use crate::{
    filter::{Callbacks, ContextToken, RawConnectionKey},
    root::fault::Faults,
};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::Sender,
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::{
    core,
    Win32::{
        Foundation::{ERROR_CLOUD_FILE_REQUEST_TIMEOUT, E_UNEXPECTED},
        Storage::CloudFilters::{CfDisconnectSyncRoot, CF_CONNECTION_KEY},
    },
};
/// Outcome of explicit native disconnect and shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownStatus {
    /// Native disconnect succeeded, no callback is executing filter code and
    /// any watcher that started joined successfully. Unregister is now safe.
    Complete,
    /// Native disconnect succeeded, but callbacks dispatched before it are
    /// still executing filter code. Windows does not drain them; poll again.
    CallbacksActive,
    /// Native disconnect succeeded and callbacks are quiescent; the watcher
    /// retains its own I/O and filter until it exits. Poll again.
    WatcherStopping,
}
/// Observable retained shutdown, separating joined watcher failure from normal
/// success. Every variant is an observation of this SAME owned connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetainedShutdown {
    /// Native disconnect succeeded, callbacks drained, any started watcher
    /// joined normally. No watcher at all is separately NotStarted metadata.
    Complete,
    /// Native disconnect succeeded; dispatched callbacks still own references.
    CallbacksActive,
    /// Native disconnect succeeded; actual watcher resources remain retained.
    WatcherStopping,
    /// Native disconnect succeeded, callbacks drained, actual watcher joined
    /// with failure. This is NOT successful watcher/provider completion.
    DrainedWithWatcherFailure,
}
/// Pure current watcher metadata; it grants no native owner or shutdown ACK.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatcherState {
    /// No watcher was ever spawned; cannot qualify provider startup.
    NotStarted,
    /// An actual retained watcher thread is not finished at this observation.
    Running,
    /// The actual watcher exited, but its join result is not yet observed.
    Exited,
    /// The actual started watcher joined normally.
    Joined,
    /// The actual watcher joined with failure; error provenance remains retained.
    FailedJoined,
}
/// Owns native callbacks and the independent attribute watcher. Drop retains
/// a bounded fallback; use [Connection::disconnect] for an observable outcome.
///
/// Lifetime contract:
/// - `CallbackContext` is an opaque registry token, never a pointer. Native
///   disconnect does not wait for dispatched callbacks, so the token is removed
///   after successful disconnect and late callbacks resolve no filter.
/// - Callbacks that resolved the filter before removal hold it alive and are
///   counted; [ShutdownStatus::Complete] is reported only once they finish.
/// - If native disconnect fails, the token stays registered: callbacks remain
///   fully functional and disconnect may be retried.
/// - The callback table is copied by Windows during connect.
#[derive(Debug)]
pub struct Connection<F> {
    connection_key: RawConnectionKey,
    cancel_token: Option<Sender<()>>,
    join_handle: Option<JoinHandle<()>>,
    watcher_started: bool,
    disconnected: bool,
    watcher_failed: bool,
    faults: Faults,
    _callbacks: Callbacks,
    filter: Arc<F>,
    context: Option<ContextToken>,
    active: Arc<AtomicUsize>,
}
impl<T> Connection<T> {
    // Construct only from the genuine successful native return in Session.
    // No watcher/channel exists yet; its failure must not discard this owner.
    pub(crate) fn new(
        connection_key: RawConnectionKey,
        callbacks: Callbacks,
        filter: Arc<T>,
        context: ContextToken,
        faults: Faults,
    ) -> Self {
        Self {
            connection_key,
            cancel_token: None,
            join_handle: None,
            watcher_started: false,
            disconnected: false,
            watcher_failed: false,
            faults,
            _callbacks: callbacks,
            filter,
            active: context.active(),
            context: Some(context),
        }
    }
    // Session transfers its one genuinely spawned watcher exactly once.
    pub(crate) fn attach_watcher(&mut self, sender: Sender<()>, handle: JoinHandle<()>) {
        self.cancel_token = Some(sender);
        self.join_handle = Some(handle);
        self.watcher_started = true;
    }
    /// Explicit, idempotent disconnect. Native errors retain full ownership for
    /// retry. Poll [ShutdownStatus::CallbacksActive] and
    /// [ShutdownStatus::WatcherStopping] until Complete before unregistering.
    /// A watcher panic remains an error on later calls. No deadline is imposed
    /// on the native CfDisconnectSyncRoot call or on application callback code;
    /// a callback that never returns keeps this in CallbacksActive.
    pub fn disconnect(&mut self) -> core::Result<ShutdownStatus> {
        match self.disconnect_retained()? {
            RetainedShutdown::Complete => Ok(ShutdownStatus::Complete),
            RetainedShutdown::CallbacksActive => Ok(ShutdownStatus::CallbacksActive),
            RetainedShutdown::WatcherStopping => Ok(ShutdownStatus::WatcherStopping),
            RetainedShutdown::DrainedWithWatcherFailure => {
                Err(core::Error::from_hresult(E_UNEXPECTED))
            }
        }
    }
    /// Explicit retained-owner shutdown. Native errors keep this actual key,
    /// context token/filter and watcher available for the original caller.
    /// No deadline or callback-count-only approximation grants completion.
    pub fn disconnect_retained(&mut self) -> core::Result<RetainedShutdown> {
        if !self.disconnected {
            if self.faults.take_disconnect_failure() {
                // Injected before the native call: the connection stays active.
                return Err(core::Error::from_hresult(
                    ERROR_CLOUD_FILE_REQUEST_TIMEOUT.to_hresult(),
                ));
            }
            unsafe { CfDisconnectSyncRoot(CF_CONNECTION_KEY(self.connection_key)) }?;
            self.disconnected = true;
            // After removal no callback can newly resolve the filter.
            self.context.take();
            if let Some(cancel) = &self.cancel_token {
                let _ = cancel.send(());
            }
        }
        if self.active_callbacks() > 0 {
            return Ok(RetainedShutdown::CallbacksActive);
        }
        if self.watcher_failed {
            return Ok(RetainedShutdown::DrainedWithWatcherFailure);
        }
        if self.join_handle.as_ref().is_some_and(|h| !h.is_finished()) {
            return Ok(RetainedShutdown::WatcherStopping);
        }
        if let Some(handle) = self.join_handle.take() {
            if handle.join().is_err() {
                self.watcher_failed = true;
                return Ok(RetainedShutdown::DrainedWithWatcherFailure);
            }
        }
        Ok(RetainedShutdown::Complete)
    }
    /// Current metadata only. Running is required before normal provider
    /// publication; NotStarted/Exited/FailedJoined cannot become readiness.
    pub fn watcher_state(&self) -> WatcherState {
        if self.watcher_failed {
            WatcherState::FailedJoined
        } else if let Some(handle) = &self.join_handle {
            if handle.is_finished() {
                WatcherState::Exited
            } else {
                WatcherState::Running
            }
        } else if self.watcher_started {
            WatcherState::Joined
        } else {
            WatcherState::NotStarted
        }
    }
    /// True until a native disconnect has succeeded. A failed disconnect leaves
    /// this true: callbacks may still run and unregister is not yet safe.
    pub fn is_connected(&self) -> bool {
        !self.disconnected
    }
    /// Number of callbacks currently executing filter code for this connection.
    pub fn active_callbacks(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }
    /// Raw connection identifier; invalid for operations after disconnect.
    pub fn connection_key(&self) -> RawConnectionKey {
        self.connection_key
    }
    /// Filter remains owned until this Connection is dropped.
    pub fn filter(&self) -> &T {
        &self.filter
    }
}
impl<T> Drop for Connection<T> {
    fn drop(&mut self) {
        if let Err(error) = self.disconnect() {
            log::error!("Cannot disconnect sync root: {error}");
            // Removing the token is safe even while natively connected: later
            // callbacks resolve no filter. Callbacks then return without
            // completing their operation and Windows times the request out.
            self.context.take();
            if let Some(cancel) = &self.cancel_token {
                let _ = cancel.send(());
            }
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while (self.active_callbacks() > 0
            || self.join_handle.as_ref().is_some_and(|h| !h.is_finished()))
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(50));
        }
        if self.active_callbacks() > 0 {
            log::warn!("Callbacks still active at drop; they retain the filter until they return");
        }
        if self.join_handle.as_ref().is_some_and(|h| !h.is_finished()) {
            log::warn!("Root watcher still stopping; its thread retains all I/O resources");
        } else if self.disconnected && !self.watcher_failed {
            if let Err(error) = self.disconnect() {
                log::error!("Root watcher shutdown failed: {error}");
            }
        }
    }
}

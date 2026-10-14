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
    /// the watcher joined successfully. Unregister is now safe.
    Complete,
    /// Native disconnect succeeded, but callbacks dispatched before it are
    /// still executing filter code. Windows does not drain them; poll again.
    CallbacksActive,
    /// Native disconnect succeeded and callbacks are quiescent; the watcher
    /// retains its own I/O and filter until it exits. Poll again.
    WatcherStopping,
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
    cancel_token: Sender<()>,
    join_handle: Option<JoinHandle<()>>,
    disconnected: bool,
    watcher_failed: bool,
    faults: Faults,
    _callbacks: Callbacks,
    filter: Arc<F>,
    context: Option<ContextToken>,
    active: Arc<AtomicUsize>,
}
impl<T> Connection<T> {
    pub(crate) fn new(
        connection_key: RawConnectionKey,
        cancel_token: Sender<()>,
        join_handle: JoinHandle<()>,
        callbacks: Callbacks,
        filter: Arc<T>,
        context: ContextToken,
        faults: Faults,
    ) -> Self {
        Self {
            connection_key,
            cancel_token,
            join_handle: Some(join_handle),
            disconnected: false,
            watcher_failed: false,
            faults,
            _callbacks: callbacks,
            filter,
            active: context.active(),
            context: Some(context),
        }
    }
    /// Explicit, idempotent disconnect. Native errors retain full ownership for
    /// retry. Poll [ShutdownStatus::CallbacksActive] and
    /// [ShutdownStatus::WatcherStopping] until Complete before unregistering.
    /// A watcher panic remains an error on later calls. No deadline is imposed
    /// on the native CfDisconnectSyncRoot call or on application callback code;
    /// a callback that never returns keeps this in CallbacksActive.
    pub fn disconnect(&mut self) -> core::Result<ShutdownStatus> {
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
            let _ = self.cancel_token.send(());
        }
        if self.watcher_failed {
            return Err(core::Error::from_hresult(E_UNEXPECTED));
        }
        if self.active_callbacks() > 0 {
            return Ok(ShutdownStatus::CallbacksActive);
        }
        if self.join_handle.as_ref().is_some_and(|h| !h.is_finished()) {
            return Ok(ShutdownStatus::WatcherStopping);
        }
        if let Some(handle) = self.join_handle.take() {
            if handle.join().is_err() {
                self.watcher_failed = true;
                return Err(core::Error::from_hresult(E_UNEXPECTED));
            }
        }
        Ok(ShutdownStatus::Complete)
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
            let _ = self.cancel_token.send(());
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

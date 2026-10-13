use crate::filter::{Callbacks, RawConnectionKey};
use std::{
    sync::{mpsc::Sender, Arc, Weak},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::{
    core,
    Win32::{
        Foundation::E_UNEXPECTED,
        Storage::CloudFilters::{CfDisconnectSyncRoot, CF_CONNECTION_KEY},
    },
};
/// Outcome of explicit native disconnect and watcher shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownStatus {
    /// Native callbacks ended and the watcher joined successfully.
    Complete,
    /// Native callbacks ended; the watcher retains its own I/O and filter.
    WatcherStopping,
}
/// Owns native callbacks and the independent attribute watcher. Drop retains
/// the prior bounded fallback; use disconnect for an observable outcome.
#[derive(Debug)]
pub struct Connection<F> {
    connection_key: RawConnectionKey,
    cancel_token: Sender<()>,
    join_handle: Option<JoinHandle<()>>,
    disconnected: bool,
    watcher_failed: bool,
    _callbacks: Callbacks,
    filter: Arc<F>,
    context: Option<Box<Weak<F>>>,
}
impl<T> Connection<T> {
    pub(crate) fn new(
        connection_key: RawConnectionKey,
        cancel_token: Sender<()>,
        join_handle: JoinHandle<()>,
        callbacks: Callbacks,
        filter: Arc<T>,
        context: Box<Weak<T>>,
    ) -> Self {
        Self {
            connection_key,
            cancel_token,
            join_handle: Some(join_handle),
            disconnected: false,
            watcher_failed: false,
            _callbacks: callbacks,
            filter,
            context: Some(context),
        }
    }
    /// Explicit, idempotent disconnect. Native errors retain full ownership for
    /// retry. Poll WatcherStopping until Complete before unregistering. A watcher
    /// panic remains an error on later calls. No deadline is imposed on the native
    /// CfDisconnectSyncRoot call or arbitrary application callback code.
    pub fn disconnect(&mut self) -> core::Result<ShutdownStatus> {
        if !self.disconnected {
            unsafe { CfDisconnectSyncRoot(CF_CONNECTION_KEY(self.connection_key)) }?;
            self.disconnected = true;
            self.context.take();
            let _ = self.cancel_token.send(());
        }
        if self.watcher_failed {
            return Err(core::Error::from_hresult(E_UNEXPECTED));
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
            if !self.disconnected {
                if let Some(context) = self.context.take() {
                    let _ = Box::into_raw(context);
                }
            }
            let _ = self.cancel_token.send(());
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.join_handle.as_ref().is_some_and(|h| !h.is_finished())
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(50));
        }
        if self.join_handle.as_ref().is_some_and(|h| !h.is_finished()) {
            log::warn!("Root watcher still stopping; its thread retains all I/O resources");
        } else if self.disconnected {
            if let Err(error) = self.disconnect() {
                log::error!("Root watcher shutdown failed: {error}");
            }
        }
    }
}

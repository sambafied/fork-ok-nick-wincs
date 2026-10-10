use std::{
    sync::{mpsc::Sender, Arc, Weak},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use windows::Win32::Storage::CloudFilters::{CfDisconnectSyncRoot, CF_CONNECTION_KEY};

use crate::filter::{Callbacks, RawConnectionKey};

/// A handle to the current session for a given sync root.
///
/// [Connection] will disconnect when dropped. Note that this
/// does **NOT** mean the sync root will be unregistered. To do so, call
/// [SyncRootId::unregister][crate::root::SyncRootId::unregister].
#[derive(Debug)]
pub struct Connection<F> {
    connection_key: RawConnectionKey,

    cancel_token: Sender<()>,
    join_handle: JoinHandle<()>,

    _callbacks: Callbacks,
    filter: Arc<F>,
    context: Option<Box<Weak<F>>>,
}

// this struct could house many more windows api functions, although they all seem to do nothing
// according to the threads on microsoft q&a
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
            join_handle,
            _callbacks: callbacks,
            filter,
            context: Some(context),
        }
    }

    /// A raw connection key used to identify the connection.
    pub fn connection_key(&self) -> RawConnectionKey {
        self.connection_key
    }

    /// A reference to the inner [SyncFilter][crate::filter::SyncFilter] struct.
    pub fn filter(&self) -> &T {
        &self.filter
    }
}

impl<T> Drop for Connection<T> {
    fn drop(&mut self) {
        if let Err(error) = unsafe { CfDisconnectSyncRoot(CF_CONNECTION_KEY(self.connection_key)) }
        {
            log::error!("Cannot disconnect sync root: {error}");
            // Failed disconnect cannot prove callback quiescence. Keep the tiny
            // immutable Weak context valid instead of exposing a dangling pointer.
            if let Some(context) = self.context.take() {
                let _ = Box::into_raw(context);
            }
        }

        _ = self.cancel_token.send(());
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.join_handle.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
        if !self.join_handle.is_finished() {
            log::warn!("Root watcher still stopping; its thread retains all I/O resources");
        }
    }
}

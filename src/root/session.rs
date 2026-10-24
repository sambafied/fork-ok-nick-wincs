use std::{
    fs::{File, OpenOptions},
    mem::{self, MaybeUninit},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, Sender, TryRecvError},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use widestring::{U16CString, U16Str};
use windows::{
    core::{self, PCWSTR},
    Win32::{
        Foundation::{ERROR_IO_INCOMPLETE, E_INVALIDARG, HANDLE, WIN32_ERROR},
        Storage::{
            CloudFilters::{self, CfConnectSyncRoot, CF_CONNECT_FLAGS},
            FileSystem::{
                ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED,
                FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_INFORMATION,
            },
        },
        System::IO::{CancelIoEx, GetOverlappedResult},
    },
};

use crate::{
    filter::{self, AsyncBridge, ContextToken, Filter, SyncFilter},
    root::{connect::Connection, fault::Faults},
    utility::LocalBoxFuture,
};

/// Error plus genuine partial native ownership. A watcher-spawn failure after
/// successful native connect retains the SAME Connection; the caller must
/// explicitly drain it. Dropping an error uses only Connection's legacy fallback.
#[must_use = "transfer the retained connection to its original shutdown owner"]
pub struct ConnectFailure<F> {
    error: core::Error,
    connection: Option<Box<Connection<F>>>,
}
impl<F> ConnectFailure<F> {
    fn before_connect(error: core::Error) -> Self {
        Self {
            error,
            connection: None,
        }
    }
    /// Original native or watcher-start error, independent of shutdown outcome.
    pub fn error(&self) -> &core::Error {
        &self.error
    }
    /// Whether this error owns a genuine successful native-connect return.
    pub fn has_connection(&self) -> bool {
        self.connection.is_some()
    }
    /// Transfers genuine ownership once; never reconstructs a native key.
    pub fn into_parts(self) -> (core::Error, Option<Connection<F>>) {
        (self.error, self.connection.map(|connection| *connection))
    }
}
impl<F> std::fmt::Debug for ConnectFailure<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectFailure")
            .field("code", &self.error.code())
            .field("retained_connection", &self.connection.is_some())
            .finish()
    }
}
impl<F> std::fmt::Display for ConnectFailure<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Native connect failed ({:#010x}); retained connection: {}",
            self.error.code().0,
            self.connection.is_some()
        )
    }
}
impl<F> std::error::Error for ConnectFailure<F> {}

/// A builder to create a new connection for the sync root at the specified path.
#[derive(Debug, Clone, Copy)]
pub struct Session(CF_CONNECT_FLAGS, Faults);

impl Session {
    /// Create a new [Session].
    pub fn new() -> Self {
        Self::default()
    }

    /// The [Session::block_implicit_hydration] flag will prevent
    /// implicit placeholder hydrations from invoking
    /// [SyncFilter::fetch_data][crate::filter::SyncFilter::fetch_data]. This could occur when an
    /// anti-virus is scanning file system activity on files within the sync root.
    ///
    /// A call to the [Placeholder::hydrate][crate::placeholder::Placeholder::hydrate] trait will not be blocked by this flag.
    pub fn block_implicit_hydration(mut self) -> Self {
        self.0 |= CloudFilters::CF_CONNECT_FLAG_BLOCK_SELF_IMPLICIT_HYDRATION;
        self
    }

    /// Binds a test-only [FaultPlan][crate::root::FaultPlan] to this session and
    /// the connection it creates.
    #[cfg(feature = "fault-injection")]
    pub fn with_faults(mut self, plan: crate::root::FaultPlan) -> Self {
        self.1 = Faults::new(plan);
        self
    }

    /// Initiates a connection to the sync root with the given [SyncFilter].
    ///
    /// A path containing NUL fails with `E_INVALIDARG`. If watcher creation
    /// fails after the native connection exists, the connection is rolled back
    /// and the callback context token is removed, so any late callback returns
    /// without reaching the filter.
    pub fn connect<P, F>(self, path: P, filter: F) -> core::Result<Connection<F>>
    where
        P: AsRef<Path>,
        F: SyncFilter + 'static,
    {
        match self.connect_retained(path, filter) {
            Ok(connection) => Ok(connection),
            Err(failure) => {
                let (error, retained) = failure.into_parts();
                if let Some(mut connection) = retained {
                    if let Err(rollback) = connection.disconnect() {
                        log::error!("Connect rollback could not disconnect: {rollback}");
                    }
                    // Legacy return type cannot transfer partial ownership.
                    // Its existing documented bounded Drop fallback remains.
                }
                Err(error)
            }
        }
    }
    /// Connect while retaining genuine partial ownership on watcher failure.
    /// No rollback/drop occurs on the retained error path. Callers must take
    /// the actual Connection from ConnectFailure and explicitly shut it down.
    pub fn connect_retained<P, F>(
        self,
        path: P,
        filter: F,
    ) -> std::result::Result<Connection<F>, ConnectFailure<F>>
    where
        P: AsRef<Path>,
        F: SyncFilter + 'static,
    {
        let native_path = U16CString::from_os_str(path.as_ref())
            .map_err(|_| ConnectFailure::before_connect(core::Error::from_hresult(E_INVALIDARG)))?;
        let directory = open_watch_directory(path.as_ref())
            .map_err(|error| ConnectFailure::before_connect(error.into()))?;
        let filter = Arc::new(filter);
        let callbacks = filter::callbacks::<F>();
        let context = ContextToken::register(&filter);
        let key = unsafe {
            CfConnectSyncRoot(
                PCWSTR(native_path.as_ptr()),
                callbacks.as_ptr(),
                Some(context.as_context()),
                self.0
                    | CloudFilters::CF_CONNECT_FLAG_REQUIRE_FULL_FILE_PATH
                    | CloudFilters::CF_CONNECT_FLAG_REQUIRE_PROCESS_INFO,
            )
        }
        .map_err(ConnectFailure::before_connect)?;
        // Establish actual key/context/callback ownership BEFORE watcher spawn.
        let mut connection = Connection::new(key.0, callbacks, filter.clone(), context, self.1);
        let watcher = if self.1.watcher_spawn() {
            Err(std::io::Error::other("injected watcher spawn failure"))
        } else {
            spawn_root_watcher(
                path.as_ref().to_path_buf(),
                directory,
                filter,
                self.1.watcher_panic(),
            )
        };
        match watcher {
            Ok((sender, handle)) => {
                connection.attach_watcher(sender, handle);
                Ok(connection)
            }
            Err(error) => Err(ConnectFailure {
                error: error.into(),
                connection: Some(Box::new(connection)),
            }),
        }
    }

    /// Initiates a connection to the sync root with the given [Filter].
    pub fn connect_async<P, F, B>(
        self,
        path: P,
        filter: F,
        block_on: B,
    ) -> core::Result<Connection<AsyncBridge<F, B>>>
    where
        P: AsRef<Path>,
        F: Filter + 'static,
        B: Fn(LocalBoxFuture<'_, ()>) + Send + Sync + 'static,
    {
        self.connect(path, AsyncBridge::new(filter, block_on))
    }
}

impl Default for Session {
    fn default() -> Self {
        Self(CloudFilters::CF_CONNECT_FLAG_NONE, Faults::default())
    }
}

fn spawn_root_watcher<T: SyncFilter + 'static>(
    path: PathBuf,
    sync_root: File,
    filter: Arc<T>,
    inject_panic: bool,
) -> std::io::Result<(Sender<()>, JoinHandle<()>)> {
    let (tx, rx) = mpsc::channel();
    let handle = thread::Builder::new()
        .name("wincs-root-watcher".into())
        .spawn(move || {
            // Injected before any overlapped I/O, so unwinding frees no buffer
            // the kernel could still write.
            if inject_panic {
                panic!("injected watcher panic");
            }
            const CHANGE_BUF_SIZE: usize = 1024;

            let mut changes_buf = MaybeUninit::<[u32; CHANGE_BUF_SIZE / 4]>::zeroed();
            let mut overlapped = MaybeUninit::zeroed();
            let mut transferred = MaybeUninit::zeroed();

            while matches!(rx.try_recv(), Err(TryRecvError::Empty)) {
                unsafe {
                    ReadDirectoryChangesW(
                        HANDLE(sync_root.as_raw_handle() as _),
                        changes_buf.as_mut_ptr() as *mut _,
                        CHANGE_BUF_SIZE as _,
                        true,
                        FILE_NOTIFY_CHANGE_ATTRIBUTES,
                        None,
                        Some(overlapped.as_mut_ptr()),
                        None,
                    )
                }
                .expect("read directory changes");

                loop {
                    if let Err(e) = unsafe {
                        GetOverlappedResult(
                            HANDLE(sync_root.as_raw_handle() as _),
                            overlapped.as_ptr(),
                            transferred.as_mut_ptr(),
                            false,
                        )
                    } {
                        if e.code() != ERROR_IO_INCOMPLETE.to_hresult() {
                            panic!(
                                "get overlapped result: {:?}, expected: {ERROR_IO_INCOMPLETE:?}",
                                WIN32_ERROR::from_error(&e),
                            );
                        }

                        // cancel by user
                        if !matches!(rx.try_recv(), Err(TryRecvError::Empty)) {
                            _ = unsafe {
                                CancelIoEx(
                                    HANDLE(sync_root.as_raw_handle() as _),
                                    Some(overlapped.as_ptr()),
                                )
                            };
                            // Cancellation is a request, not completion. Keep the
                            // OVERLAPPED and buffer alive until the kernel is done.
                            let _ = unsafe {
                                GetOverlappedResult(
                                    HANDLE(sync_root.as_raw_handle() as _),
                                    overlapped.as_ptr(),
                                    transferred.as_mut_ptr(),
                                    true,
                                )
                            };
                            return;
                        }

                        thread::sleep(Duration::from_millis(300));
                        continue;
                    }

                    if unsafe { transferred.assume_init() } == 0 {
                        break;
                    }

                    let mut changes = Vec::with_capacity(8);
                    let mut entry = changes_buf.as_ptr() as *const FILE_NOTIFY_INFORMATION;
                    loop {
                        let relative = unsafe {
                            U16Str::from_ptr(
                                &(*entry).FileName as *const _,
                                (*entry).FileNameLength as usize / mem::size_of::<u16>(),
                            )
                        };

                        changes.push(path.join(relative.to_os_string()));

                        if unsafe { *entry }.NextEntryOffset == 0 {
                            break;
                        }
                        entry = unsafe { entry.byte_add((*entry).NextEntryOffset as _) };
                    }

                    filter.state_changed(changes);
                    break;
                }
            }
        })?;

    Ok((tx, handle))
}

fn open_watch_directory(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY.0)
        .custom_flags((FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED).0)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        error::{CResult, CloudErrorKind},
        filter::{info, ticket, Request},
    };
    struct EmptyFilter;
    impl SyncFilter for EmptyFilter {
        fn fetch_data(&self, _: Request, _: ticket::FetchData, _: info::FetchData) -> CResult<()> {
            Err(CloudErrorKind::NotSupported)
        }
    }
    #[test]
    fn idle_native_directory_watcher_cancels_and_releases_directory() {
        let directory = tempfile::tempdir().unwrap();
        let file = open_watch_directory(directory.path()).unwrap();
        let (stop, watcher) =
            spawn_root_watcher(directory.path().into(), file, Arc::new(EmptyFilter), false)
                .unwrap();
        thread::sleep(Duration::from_millis(100));
        stop.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !watcher.is_finished() && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            watcher.is_finished(),
            "idle watcher cancellation exceeded deadline"
        );
        watcher.join().unwrap();
        directory.close().unwrap();
    }
}

use std::{
    fs::{File, OpenOptions},
    mem::{self, MaybeUninit},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, Sender, TryRecvError},
        Arc, Weak,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use widestring::{U16CString, U16Str};
use windows::{
    core::{self, PCWSTR},
    Win32::{
        Foundation::{ERROR_IO_INCOMPLETE, HANDLE, WIN32_ERROR},
        Storage::{
            CloudFilters::{
                self, CfConnectSyncRoot, CfDisconnectSyncRoot, CF_CONNECTION_KEY, CF_CONNECT_FLAGS,
            },
            FileSystem::{
                ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED,
                FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_ATTRIBUTES, FILE_NOTIFY_INFORMATION,
            },
        },
        System::IO::{CancelIoEx, GetOverlappedResult},
    },
};

use crate::{
    filter::{self, AsyncBridge, Filter, SyncFilter},
    root::connect::Connection,
    utility::LocalBoxFuture,
};

/// A builder to create a new connection for the sync root at the specified path.
#[derive(Debug, Clone, Copy)]
pub struct Session(CF_CONNECT_FLAGS);

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

    /// Initiates a connection to the sync root with the given [SyncFilter].
    pub fn connect<P, F>(self, path: P, filter: F) -> core::Result<Connection<F>>
    where
        P: AsRef<Path>,
        F: SyncFilter + 'static,
    {
        // Verify/open the watcher directory before connecting the provider.
        let directory = open_watch_directory(path.as_ref())?;
        let filter = Arc::new(filter);
        let callbacks = filter::callbacks::<F>();
        let context = Box::new(Arc::downgrade(&filter));
        let key = unsafe {
            CfConnectSyncRoot(
                PCWSTR(
                    U16CString::from_os_str(path.as_ref())
                        .expect("not contains nul")
                        .as_ptr(),
                ),
                callbacks.as_ptr(),
                Some(context.as_ref() as *const Weak<F> as *const _),
                // This is enabled by default to remove the Option requirement around various fields of the
                // [Request][crate::Request] struct
                self.0
                    | CloudFilters::CF_CONNECT_FLAG_REQUIRE_FULL_FILE_PATH
                    | CloudFilters::CF_CONNECT_FLAG_REQUIRE_PROCESS_INFO,
            )
        }?;

        let (cancel_token, join_handle) =
            match spawn_root_watcher(path.as_ref().to_path_buf(), directory, filter.clone()) {
                Ok(watcher) => watcher,
                Err(error) => {
                    if unsafe { CfDisconnectSyncRoot(CF_CONNECTION_KEY(key.0)) }.is_err() {
                        let _ = Box::into_raw(context);
                    }
                    return Err(error.into());
                }
            };

        Ok(Connection::new(
            key.0,
            cancel_token,
            join_handle,
            callbacks,
            filter,
            context,
        ))
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
        Self(CloudFilters::CF_CONNECT_FLAG_NONE)
    }
}

fn spawn_root_watcher<T: SyncFilter + 'static>(
    path: PathBuf,
    sync_root: File,
    filter: Arc<T>,
) -> std::io::Result<(Sender<()>, JoinHandle<()>)> {
    let (tx, rx) = mpsc::channel();
    let handle = thread::Builder::new()
        .name("wincs-root-watcher".into())
        .spawn(move || {
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
            spawn_root_watcher(directory.path().into(), file, Arc::new(EmptyFilter)).unwrap();
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

use anyhow::{ensure, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use wincs::{
    error::{CResult, CloudErrorKind},
    filter::{info, ticket, Filter, Request, SyncFilter},
    metadata::Metadata,
    placeholder_file::PlaceholderFile,
    root::Session,
};

struct PagedFilter {
    root: PathBuf,
    calls: Arc<AtomicUsize>,
    pages: Arc<AtomicUsize>,
}

type LocalBoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + 'a>>;
impl SyncFilter for PagedFilter {
    fn fetch_data(&self, _: Request, _: ticket::FetchData, _: info::FetchData) -> CResult<()> {
        Err(CloudErrorKind::NotSupported)
    }
    fn fetch_placeholders(
        &self,
        request: Request,
        ticket: ticket::FetchPlaceholders,
        _: info::FetchPlaceholders,
    ) -> CResult<()> {
        let path = request.path();
        println!("population callback: {}", path.display());
        let relative = path.strip_prefix(&self.root).map_err(|error| {
            eprintln!("population operation failed: {error}");
            CloudErrorKind::InvalidRequest
        })?;
        let mut entries = match relative.to_str() {
            Some("") => vec![
                PlaceholderFile::new("empty")
                    .mark_in_sync()
                    .metadata(
                        Metadata::directory()
                            .created(nt_time::FileTime::now())
                            .written(nt_time::FileTime::now()),
                    )
                    .blob(b"empty".to_vec()),
                PlaceholderFile::new("paged")
                    .mark_in_sync()
                    .metadata(
                        Metadata::directory()
                            .created(nt_time::FileTime::now())
                            .written(nt_time::FileTime::now()),
                    )
                    .blob(b"paged".to_vec()),
            ],
            Some("empty") => vec![],
            Some("paged") => (0..128)
                .map(|i| {
                    PlaceholderFile::new(format!("item-{i:03}.txt"))
                        .mark_in_sync()
                        .metadata(Metadata::file().size(0))
                        .blob(b"empty-fixture".to_vec())
                })
                .collect(),
            _ => return Err(CloudErrorKind::InvalidRequest),
        };
        let total = entries.len() as u64;
        let mut processed_total = 0;
        // Explicit empty completion, not a silent successful return that hangs enumeration.
        if entries.is_empty() {
            let processed = ticket.pass_batch(&mut [], 0, true).map_err(|error| {
                eprintln!("population operation failed: {error}");
                CloudErrorKind::InvalidRequest
            })?;
            if processed != 0 {
                return Err(CloudErrorKind::InvalidRequest);
            }
        }
        let count = entries.len().div_ceil(32);
        for (index, batch) in entries.chunks_mut(32).enumerate() {
            let processed = ticket
                .pass_batch(batch, total, index + 1 == count)
                .map_err(|error| {
                    eprintln!("population operation failed: {error}");
                    CloudErrorKind::InvalidRequest
                })?;
            if processed as usize != batch.len()
                || batch.iter().any(|entry| entry.result().is_err())
            {
                return Err(CloudErrorKind::InvalidRequest);
            }
            processed_total += processed;
            if relative == Path::new("paged") {
                self.pages.fetch_add(1, Ordering::SeqCst);
            }
        }
        println!(
            "population {}: {processed_total}/{total} processed in {count} pages",
            relative.display()
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl Filter for PagedFilter {
    async fn fetch_data(
        &self,
        _: Request,
        _: ticket::FetchData,
        _: info::FetchData,
    ) -> CResult<()> {
        Err(CloudErrorKind::NotSupported)
    }
    async fn fetch_placeholders(
        &self,
        request: Request,
        ticket: ticket::FetchPlaceholders,
        info: info::FetchPlaceholders,
    ) -> CResult<()> {
        SyncFilter::fetch_placeholders(self, request, ticket, info)
    }
}

fn execute(future: LocalBoxFuture<'_, ()>) {
    futures::executor::block_on(future);
}
type Executor = for<'a> fn(LocalBoxFuture<'a, ()>);

pub fn test(async_mode: bool) -> anyhow::Result<()> {
    let mut owned = crate::owned_root::OwnedRoot::new()?;
    let calls = Arc::new(AtomicUsize::new(0));
    let pages = Arc::new(AtomicUsize::new(0));
    let filter = PagedFilter {
        root: owned.path.clone(),
        calls: calls.clone(),
        pages: pages.clone(),
    };
    if async_mode {
        let connection = Session::new().connect_async(&owned.path, filter, execute as Executor)?;
        crate::run_client_mode(&owned.path, "--verify-paged-client")?;
        drop(connection);
    } else {
        let connection = Session::new().connect(&owned.path, filter)?;
        crate::run_client_mode(&owned.path, "--verify-paged-client")?;
        drop(connection);
    }
    ensure!(
        calls.load(Ordering::SeqCst) == 3,
        "missing root/empty/paged callback"
    );
    ensure!(
        pages.load(Ordering::SeqCst) == 4,
        "missing intermediate pages"
    );
    owned.finish()?;
    Ok(())
}

pub fn verify_client(root: &Path) -> Result<()> {
    let mut names = fs::read_dir(root)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    names.sort();
    ensure!(
        names == ["empty", "paged"],
        "root entries mismatch: {names:?}"
    );
    ensure!(
        fs::read_dir(root.join("empty"))?.next().is_none(),
        "empty directory not empty"
    );
    let mut names = fs::read_dir(root.join("paged"))?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?;
    names.sort();
    let expected: Vec<_> = (0..128)
        .map(|i| std::ffi::OsString::from(format!("item-{i:03}.txt")))
        .collect();
    ensure!(
        names == expected,
        "paged listing truncated or changed: {} entries",
        names.len()
    );
    // A second enumeration verifies the completed listing persists locally.
    ensure!(
        fs::read_dir(root.join("paged"))?.count() == 128,
        "completed listing lost"
    );
    Ok(())
}

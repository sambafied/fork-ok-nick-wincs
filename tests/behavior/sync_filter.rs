use core::str;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use anyhow::Context;
use nt_time::FileTime;
use wincs::{
    error::{CResult, CloudErrorKind},
    filter::{info, ticket, Request, SyncFilter},
    metadata::Metadata,
    placeholder_file::PlaceholderFile,
    root::Session,
    utility::WriteAt,
};

struct MemFilter {
    root: std::path::PathBuf,
    populations: Arc<AtomicUsize>,
    hydrations: Arc<AtomicUsize>,
}

impl SyncFilter for MemFilter {
    fn fetch_data(
        &self,
        request: Request,
        ticket: ticket::FetchData,
        info: info::FetchData,
    ) -> CResult<()> {
        let path =
            str::from_utf8(request.file_blob()).map_err(|_| CloudErrorKind::InvalidRequest)?;
        println!("fetch_data: path: {path:?}");

        let content = match path {
            "test1.txt" | "dir1\\test2.txt" => path,
            _ => Err(CloudErrorKind::InvalidRequest)?,
        };

        if info.required_file_range() != (0..content.len() as u64) {
            Err(CloudErrorKind::InvalidRequest)?;
        }

        ticket
            .write_at(content.as_bytes(), 0)
            .map_err(|_| CloudErrorKind::InvalidRequest)?;
        self.hydrations.fetch_add(1, Ordering::SeqCst);

        Ok(())
    }

    fn fetch_placeholders(
        &self,
        request: Request,
        ticket: ticket::FetchPlaceholders,
        _info: info::FetchPlaceholders,
    ) -> CResult<()> {
        let path = request.path();
        let relative_path = path
            .strip_prefix(&self.root)
            .map_err(|_| CloudErrorKind::InvalidRequest)?;
        println!("fetch_placeholders: path: {path:?}, relative path: {relative_path:?}");

        let now = FileTime::now();
        let mut placeholders = match relative_path.to_string_lossy().as_ref() {
            "" => vec![
                PlaceholderFile::new("dir1")
                    .mark_in_sync()
                    .metadata(Metadata::directory().created(now).written(now).size(0))
                    .blob("dir1".into()),
                PlaceholderFile::new("test1.txt")
                    .has_no_children()
                    .mark_in_sync()
                    .metadata(
                        Metadata::file()
                            .created(now)
                            .written(now)
                            .size("test1.txt".len() as _),
                    )
                    .blob("test1.txt".into()),
            ],
            "dir1" => vec![PlaceholderFile::new("test2.txt")
                .has_no_children()
                .mark_in_sync()
                .metadata(
                    Metadata::file()
                        .created(now)
                        .written(now)
                        .size("dir1\\test2.txt".len() as _),
                )
                .blob("dir1\\test2.txt".into())],
            _ => Err(CloudErrorKind::InvalidRequest)?,
        };

        let count = placeholders.len() as u64;
        let processed = ticket
            .pass_batch(&mut placeholders, count, true)
            .map_err(|_| CloudErrorKind::InvalidRequest)?;
        if u64::from(processed) != count || placeholders.iter().any(|p| p.result().is_err()) {
            return Err(CloudErrorKind::InvalidRequest);
        }
        self.populations.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

pub fn test() -> anyhow::Result<()> {
    let mut owned = crate::owned_root::OwnedRoot::new()?;
    let populations = Arc::new(AtomicUsize::new(0));
    let hydrations = Arc::new(AtomicUsize::new(0));
    let filter = MemFilter {
        root: owned.path.clone(),
        populations: populations.clone(),
        hydrations: hydrations.clone(),
    };
    let connection = Session::new().connect(&owned.path, filter)?;
    crate::run_client(owned.path.to_str().context("unicode root")?)?;
    anyhow::ensure!(
        populations.load(Ordering::SeqCst) == 2,
        "missing population callbacks"
    );
    anyhow::ensure!(
        hydrations.load(Ordering::SeqCst) == 2,
        "missing hydration callbacks"
    );
    drop(connection);
    owned.finish()?;
    Ok(())
}

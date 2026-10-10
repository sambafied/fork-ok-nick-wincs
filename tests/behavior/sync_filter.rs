use core::str;
use std::{fs, path::Path};

use anyhow::Context;
use libtest_mimic::Failed;
use nt_time::FileTime;
use wincs::{
    error::{CResult, CloudErrorKind},
    filter::{info, ticket, Request, SyncFilter},
    metadata::Metadata,
    placeholder_file::PlaceholderFile,
    root::{
        Connection, HydrationType, PopulationType, SecurityId, Session, SyncRootId,
        SyncRootIdBuilder, SyncRootInfo,
    },
    utility::WriteAt,
};

struct MemFilter {
    root: std::path::PathBuf,
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

        ticket.write_at(content.as_bytes(), 0).unwrap();

        Ok(())
    }

    fn fetch_placeholders(
        &self,
        request: Request,
        ticket: ticket::FetchPlaceholders,
        _info: info::FetchPlaceholders,
    ) -> CResult<()> {
        let path = request.path();
        let relative_path = path.strip_prefix(&self.root).unwrap();
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

        ticket.pass_with_placeholder(&mut placeholders).unwrap();
        Ok(())
    }
}

fn init(root: &str) -> anyhow::Result<(SyncRootId, Connection<MemFilter>)> {
    let sync_root_id = SyncRootIdBuilder::new("sync_filter_test_provider")
        .user_security_id(SecurityId::current_user().context("current_user")?)
        .account_name(Path::new(root).file_name().context("root name")?)
        .build();

    if sync_root_id.is_registered().context("is_registered")? {
        anyhow::bail!("Owned test identity unexpectedly exists; no registration adopted");
    }
    {
        sync_root_id
            .register(
                SyncRootInfo::default()
                    .with_display_name("Sync Filter Test")
                    .with_hydration_type(HydrationType::Full)
                    .with_population_type(PopulationType::Full)
                    .with_icon("%SystemRoot%\\system32\\charmap.exe,0")
                    .with_version("1.0.0")
                    .with_recycle_bin_uri("http://cloudmirror.example.com/recyclebin")
                    .context("recycle_bin_uri")?
                    .with_path(root)
                    .context("path")?,
            )
            .context("register")?
    }

    let connection = Session::new()
        .connect(root, MemFilter { root: root.into() })
        .context("connect")?;

    Ok((sync_root_id, connection))
}

pub fn test() -> Result<(), Failed> {
    // Fresh owned directory and unique account identity; never adopt a root.
    // Retain the directory on failures for diagnosis.
    let owned = tempfile::Builder::new()
        .prefix("sambafied-cf-test-")
        .tempdir()
        .context("owned root")?
        .keep();
    let root = owned.to_str().context("root unicode")?;

    let (sync_root_id, connection) = init(root).context("init")?;

    crate::test_list_folders(root);
    crate::test_read_file(root);

    drop(connection);
    sync_root_id.unregister().context("unregister")?;

    fs::remove_dir_all(root).context("remove root dir")?;

    Ok(())
}

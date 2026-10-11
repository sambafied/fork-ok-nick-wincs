use anyhow::{ensure, Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};
use wincs::root::{
    HydrationType, PopulationType, SecurityId, SyncRootId, SyncRootIdBuilder, SyncRootInfo,
};

/// Owns registration, not fixture deletion. Declared before Connection so unwind
/// drops the connection first. Normal finish reports cleanup failures to the test.
pub struct OwnedRoot {
    pub path: PathBuf,
    pub id: SyncRootId,
    registered: bool,
}

impl OwnedRoot {
    pub fn new() -> Result<Self> {
        let parent = PathBuf::from(
            std::env::var_os("SAMBAFIED_CF_TEST_PARENT")
                .context("SAMBAFIED_CF_TEST_PARENT must name an owned NTFS parent")?,
        );
        ensure!(
            parent.is_absolute() && parent.is_dir(),
            "absolute existing parent required"
        );
        let path = tempfile::Builder::new()
            .prefix("sambafied-cf-test-")
            .tempdir_in(parent)?
            .keep();
        let id = SyncRootIdBuilder::new("sync_filter_test_provider")
            .user_security_id(SecurityId::current_user()?)
            .account_name(path.file_name().context("account")?)
            .build();
        ensure!(!id.is_registered()?, "refusing existing identity");
        let mut owned = Self {
            path,
            id,
            registered: false,
        };
        owned.id.register(
            SyncRootInfo::default()
                .with_display_name("Sambafied isolated native test")
                .with_hydration_type(HydrationType::Full)
                .with_population_type(PopulationType::Full)
                .with_supported_attribute(
                    wincs::root::SupportedAttribute::FileLastWriteTime
                        | wincs::root::SupportedAttribute::FileCreationTime,
                )
                .with_icon("%SystemRoot%\\system32\\charmap.exe,0")
                .with_version("1.0.0")
                .with_path(&owned.path)?,
        )?;
        owned.registered = true;
        println!("registered owned fixture: {}", owned.path.display());
        Ok(owned)
    }

    pub fn finish(&mut self) -> Result<()> {
        if self.registered {
            ensure!(
                self.id.info()?.path() == self.path,
                "registered path changed; refusing cleanup"
            );
            self.id.unregister()?;
            ensure!(!self.id.is_registered()?, "registration remains");
            self.registered = false;
            println!(
                "registration absent; retained fixture: {}",
                self.path.display()
            );
        }
        Ok(())
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            eprintln!(
                "Retained registration requires recovery at {}: {error:#}",
                self.path.display()
            );
        }
    }
}

/// Recovery for the single early diagnostic failure created in this session.
pub fn cleanup_failed(root: &Path) -> Result<()> {
    ensure!(
        root.is_absolute() && root.is_dir(),
        "absolute retained root required"
    );
    let marker = fs::read_to_string(root.parent().context("parent")?.join("OWNER.txt"))?;
    ensure!(
        marker.starts_with("Issue 13/14 owned native fixtures"),
        "parent ownership marker mismatch"
    );
    let account = root.file_name().context("account")?;
    ensure!(
        account.to_string_lossy().starts_with("sambafied-cf-test-"),
        "unexpected account"
    );
    let id = SyncRootIdBuilder::new("sync_filter_test_provider")
        .user_security_id(SecurityId::current_user()?)
        .account_name(account)
        .build();
    ensure!(id.info()?.path() == root, "registered path mismatch");
    id.unregister()?;
    ensure!(!id.is_registered()?, "registration remains");
    println!("early diagnostic registration absent: {}", root.display());
    Ok(())
}

/// Read-only final cleanup check for all unique fixtures inside an owned parent.
pub fn verify_absent(parent: &Path) -> Result<()> {
    ensure!(
        parent.is_absolute() && parent.is_dir(),
        "absolute owned parent required"
    );
    let marker = fs::read_to_string(parent.join("OWNER.txt"))?;
    ensure!(
        marker.starts_with("Issue 13/14 owned native fixtures"),
        "ownership marker mismatch"
    );
    let mut checked = 0;
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir()
            || !entry
                .file_name()
                .to_string_lossy()
                .starts_with("sambafied-cf-test-")
        {
            continue;
        }
        let id = SyncRootIdBuilder::new("sync_filter_test_provider")
            .user_security_id(SecurityId::current_user()?)
            .account_name(entry.file_name())
            .build();
        ensure!(
            !id.is_registered()?,
            "owned registration persists: {}",
            entry.path().display()
        );
        checked += 1;
    }
    println!("verified {checked} exact owned registrations absent");
    Ok(())
}

pub fn cleanup_after_unwind() -> anyhow::Result<()> {
    let owned = OwnedRoot::new()?;
    let id = owned.id.clone();
    let path = owned.path.clone();
    let connection = wincs::root::Session::new().connect(&path, EmptyFilter)?;
    fs::write(path.join("retained.txt"), b"retain after unwind").context("write unwind fixture")?;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _owned = owned;
        let _connection = connection;
        panic!("injected test failure outside native callback");
    }));
    ensure!(result.is_err(), "injection did not run");
    ensure!(!id.is_registered()?, "unwind leaked registration");
    ensure!(
        fs::read(path.join("retained.txt"))? == b"retain after unwind",
        "fixture bytes lost"
    );
    Ok(())
}

pub struct EmptyFilter;
impl wincs::filter::SyncFilter for EmptyFilter {
    fn fetch_placeholders(
        &self,
        _: wincs::filter::Request,
        ticket: wincs::filter::ticket::FetchPlaceholders,
        _: wincs::filter::info::FetchPlaceholders,
    ) -> wincs::error::CResult<()> {
        // This isolated provider has no remote children. Local ordinary fixtures
        // already present remain intact; this does not seed expected test bytes.
        ticket
            .pass_batch(&mut [], 0, true)
            .map(|_| ())
            .map_err(|_| wincs::error::CloudErrorKind::Unsuccessful)
    }
    fn fetch_data(
        &self,
        _: wincs::filter::Request,
        _: wincs::filter::ticket::FetchData,
        _: wincs::filter::info::FetchData,
    ) -> wincs::error::CResult<()> {
        Err(wincs::error::CloudErrorKind::NotSupported)
    }
}

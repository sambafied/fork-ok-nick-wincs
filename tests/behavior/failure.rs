use anyhow::{ensure, Result};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use wincs::{
    error::{CResult, CloudErrorKind},
    filter::{info, ticket, Request, SyncFilter},
    metadata::Metadata,
    placeholder_file::PlaceholderFile,
    root::{Session, ShutdownStatus},
};
struct Filter(Arc<AtomicUsize>);
impl SyncFilter for Filter {
    fn fetch_data(&self, _: Request, _: ticket::FetchData, _: info::FetchData) -> CResult<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(CloudErrorKind::AuthenticationFailed)
    }
    fn fetch_placeholders(
        &self,
        _: Request,
        ticket: ticket::FetchPlaceholders,
        _: info::FetchPlaceholders,
    ) -> CResult<()> {
        ticket
            .pass_batch(&mut [], 0, true)
            .map(|_| ())
            .map_err(|_| CloudErrorKind::Unsuccessful)
    }
}
pub fn test() -> Result<()> {
    let mut root = crate::owned_root::OwnedRoot::new()?;
    let calls = Arc::new(AtomicUsize::new(0));
    let mut connection = Session::new().connect(&root.path, Filter(calls.clone()))?;
    PlaceholderFile::new("failure.bin")
        .metadata(Metadata::file().size(131089))
        .blob(vec![1])
        .create(&root.path)?;
    let start = Instant::now();
    let mut client = std::process::Command::new(std::env::current_exe()?)
        .arg("--expect-read-failure")
        .arg(root.path.join("failure.bin"))
        .spawn()?;
    loop {
        if let Some(status) = client.try_wait()? {
            ensure!(
                status.success(),
                "failed callback unexpectedly returned bytes"
            );
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            client.kill()?;
            client.wait()?;
            anyhow::bail!("failed recall did not complete within five seconds");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    ensure!(
        calls.load(Ordering::SeqCst) > 0,
        "failure did not exercise hydration callback"
    );
    println!("failed recall returned error in {:?}", start.elapsed());
    let deadline = Instant::now() + Duration::from_secs(6);
    while connection.disconnect()? != ShutdownStatus::Complete {
        ensure!(Instant::now() < deadline, "watcher still stopping");
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(connection);
    root.finish()?;
    Ok(())
}

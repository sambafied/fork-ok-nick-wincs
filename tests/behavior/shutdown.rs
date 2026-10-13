use anyhow::{ensure, Result};
use wincs::{
    error::{CResult, CloudErrorKind},
    filter::{info, ticket, Request, SyncFilter},
    root::{Session, ShutdownStatus},
};
struct Filter;
impl SyncFilter for Filter {
    fn fetch_data(&self, _: Request, _: ticket::FetchData, _: info::FetchData) -> CResult<()> {
        Err(CloudErrorKind::NotSupported)
    }
}
pub fn test() -> Result<()> {
    let mut root = crate::owned_root::OwnedRoot::new()?;
    let mut connection = Session::new().connect(&root.path, Filter)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    while connection.disconnect()? != ShutdownStatus::Complete {
        ensure!(std::time::Instant::now() < deadline, "watcher did not stop");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    ensure!(
        connection.disconnect()? == ShutdownStatus::Complete,
        "repeated close changed outcome"
    );
    let mut next = Session::new().connect(&root.path, Filter)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    while next.disconnect()? != ShutdownStatus::Complete {
        ensure!(
            std::time::Instant::now() < deadline,
            "reconnect watcher did not stop"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    drop(next);
    drop(connection);
    root.finish()?;
    Ok(())
}

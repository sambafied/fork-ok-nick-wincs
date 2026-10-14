//! Issue #7 failure/recovery matrix: in-flight callback lifetime across
//! disconnect, connect rollback, failed native disconnect and watcher panic.
use anyhow::{bail, ensure, Result};
use std::{
    path::Path,
    process::{Child, Command},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};
use wincs::{
    error::{CResult, CloudErrorKind},
    filter::{info, ticket, Request, SyncFilter},
    metadata::Metadata,
    placeholder_file::PlaceholderFile,
    root::{Connection, Session, ShutdownStatus},
};

/// Hydration callback hold time used to widen the in-flight window.
const HOLD: Duration = Duration::from_millis(1500);

/// Records callback entry/exit so disconnect ordering can be observed.
struct SlowFilter {
    entered: Mutex<mpsc::Sender<()>>,
    exits: Mutex<Vec<Instant>>,
    calls: AtomicUsize,
}
impl SyncFilter for SlowFilter {
    fn fetch_data(&self, _: Request, _: ticket::FetchData, _: info::FetchData) -> CResult<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Ok(sender) = self.entered.lock() {
            let _ = sender.send(());
        }
        std::thread::sleep(HOLD);
        if let Ok(mut exits) = self.exits.lock() {
            exits.push(Instant::now());
        }
        Err(CloudErrorKind::Unsuccessful)
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

/// Fails every recall immediately and counts invocations.
struct CountingFilter(Arc<AtomicUsize>);
impl SyncFilter for CountingFilter {
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

/// Creates a cloud-only placeholder that must be recalled through fetch_data.
fn placeholder(root: &Path, name: &str) -> Result<()> {
    PlaceholderFile::new(name)
        .metadata(Metadata::file().size(131089))
        .blob(vec![1])
        .create(root)?;
    Ok(())
}

/// Starts a separate process that expects the read to fail.
fn failing_reader(path: &Path) -> Result<Child> {
    Ok(Command::new(std::env::current_exe()?)
        .arg("--expect-read-failure")
        .arg(path)
        .spawn()?)
}

/// Waits for a reader to exit successfully within the deadline.
fn finish_reader(mut child: Child, deadline: Duration) -> Result<()> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(
                status.success(),
                "reader unexpectedly obtained bytes: {status}"
            );
            return Ok(());
        }
        if start.elapsed() > deadline {
            child.kill()?;
            child.wait()?;
            bail!("reader did not complete within {deadline:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Polls explicit disconnect until the watcher has joined.
fn close<F>(connection: &mut Connection<F>) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(6);
    while connection.disconnect()? != ShutdownStatus::Complete {
        ensure!(Instant::now() < deadline, "watcher did not stop");
        std::thread::sleep(Duration::from_millis(20));
    }
    ensure!(
        !connection.is_connected(),
        "state still connected after close"
    );
    Ok(())
}

/// Characterizes native disconnect against in-flight callbacks and qualifies
/// the wrapper contract: Complete is reported only after every callback that
/// resolved the filter has returned.
pub fn disconnect_waits_for_inflight() -> Result<()> {
    let mut root = crate::owned_root::OwnedRoot::new()?;
    let (sender, entered) = mpsc::channel();
    let filter = SlowFilter {
        entered: Mutex::new(sender),
        exits: Mutex::new(Vec::new()),
        calls: AtomicUsize::new(0),
    };
    let mut connection = Session::new().connect(&root.path, filter)?;
    placeholder(&root.path, "first.bin")?;
    placeholder(&root.path, "second.bin")?;
    let first = failing_reader(&root.path.join("first.bin"))?;
    let second = failing_reader(&root.path.join("second.bin"))?;
    entered
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| anyhow::anyhow!("no recall callback started"))?;
    // Give a concurrent second callback the chance to start as well.
    let in_flight = 1 + usize::from(entered.recv_timeout(Duration::from_millis(500)).is_ok());

    let exits_now = |connection: &Connection<SlowFilter>| -> Result<Vec<Instant>> {
        Ok(connection
            .filter()
            .exits
            .lock()
            .map_err(|_| anyhow::anyhow!("exit log poisoned"))?
            .clone())
    };
    let started = Instant::now();
    let first_status = connection.disconnect()?;
    let returned = Instant::now();
    let early = exits_now(&connection)?.len() < in_flight;
    println!(
        "NATIVE: in-flight callbacks {in_flight}; first disconnect returned after {:?} with {} still running; status {first_status:?}",
        returned - started,
        if early { in_flight - exits_now(&connection)?.len() } else { 0 }
    );
    if early {
        ensure!(
            first_status == ShutdownStatus::CallbacksActive,
            "callbacks still running but status was {first_status:?}"
        );
        ensure!(
            connection.active_callbacks() > 0,
            "active count missed running callbacks"
        );
    }
    ensure!(!connection.is_connected(), "native disconnect not recorded");
    // Poll to Complete; it must not precede any callback exit.
    let deadline = Instant::now() + HOLD + Duration::from_secs(6);
    let completed = loop {
        match connection.disconnect()? {
            ShutdownStatus::Complete => break Instant::now(),
            _ => {
                ensure!(Instant::now() < deadline, "shutdown never completed");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    };
    let exits = exits_now(&connection)?;
    ensure!(
        exits.len() >= in_flight,
        "Complete reported while {} callback(s) were running",
        in_flight - exits.len()
    );
    ensure!(
        exits.iter().all(|exit| *exit <= completed),
        "a callback finished after Complete"
    );
    ensure!(
        connection.active_callbacks() == 0,
        "active count not drained"
    );
    println!("Complete {:?} after first disconnect", completed - returned);
    // A callback finishing after disconnect cannot complete its operation, so
    // readers wait for the platform request timeout. Bound and record it.
    let waited = Instant::now();
    finish_reader(first, Duration::from_secs(90))?;
    finish_reader(second, Duration::from_secs(90))?;
    println!(
        "NATIVE: readers of callbacks orphaned by disconnect returned after {:?}",
        waited.elapsed() + (completed - returned)
    );
    ensure!(
        connection.disconnect()? == ShutdownStatus::Complete,
        "repeated close changed outcome"
    );
    let calls = connection.filter().calls.load(Ordering::SeqCst);
    ensure!(calls >= in_flight, "callback count regressed");
    drop(connection);

    // Reconnect after an in-flight close.
    let mut next = Session::new().connect(&root.path, CountingFilter(Arc::default()))?;
    close(&mut next)?;
    drop(next);
    root.finish()?;
    Ok(())
}

/// Watcher spawn failure after native connect must roll back the connection.
#[cfg(feature = "fault-injection")]
pub fn rollback_after_watcher_spawn_failure() -> Result<()> {
    use wincs::root::FaultPlan;
    let mut root = crate::owned_root::OwnedRoot::new()?;
    // Control: a second connect while connected must fail, otherwise a later
    // successful connect would not prove rollback.
    let mut control = Session::new().connect(&root.path, CountingFilter(Arc::default()))?;
    ensure!(
        Session::new()
            .connect(&root.path, CountingFilter(Arc::default()))
            .is_err(),
        "control: duplicate connect unexpectedly succeeded; rollback is unobservable"
    );
    close(&mut control)?;
    drop(control);

    let plan = FaultPlan {
        fail_watcher_spawn: true,
        ..FaultPlan::default()
    };
    match Session::new()
        .with_faults(plan)
        .connect(&root.path, CountingFilter(Arc::default()))
    {
        Ok(_) => bail!("injected watcher spawn failure did not fail connect"),
        Err(error) => println!("injected spawn failure reported: {error}"),
    }
    let mut after = Session::new().connect(&root.path, CountingFilter(Arc::default()))?;
    close(&mut after)?;
    drop(after);
    root.finish()?;
    Ok(())
}

/// A failed native disconnect keeps callbacks live and permits retry.
#[cfg(feature = "fault-injection")]
pub fn failed_disconnect_retains_callbacks() -> Result<()> {
    use wincs::root::FaultPlan;
    use windows::Win32::Foundation::ERROR_CLOUD_FILE_REQUEST_TIMEOUT;
    let mut root = crate::owned_root::OwnedRoot::new()?;
    let calls = Arc::new(AtomicUsize::new(0));
    let plan = FaultPlan {
        fail_disconnects: 2,
        ..FaultPlan::default()
    };
    let mut connection = Session::new()
        .with_faults(plan)
        .connect(&root.path, CountingFilter(calls.clone()))?;
    placeholder(&root.path, "retained.bin")?;
    for attempt in 1..=2 {
        match connection.disconnect() {
            Err(error) => ensure!(
                error.code() == ERROR_CLOUD_FILE_REQUEST_TIMEOUT.to_hresult(),
                "attempt {attempt}: unexpected failure {error}"
            ),
            Ok(status) => bail!("attempt {attempt}: injected failure returned {status:?}"),
        }
        ensure!(connection.is_connected(), "failed disconnect cleared state");
    }
    // Callbacks still reach the retained context and complete promptly.
    let start = Instant::now();
    finish_reader(
        failing_reader(&root.path.join("retained.bin"))?,
        Duration::from_secs(5),
    )?;
    ensure!(
        calls.load(Ordering::SeqCst) > 0,
        "callback did not run after failed disconnect"
    );
    println!(
        "recall after failed disconnect completed in {:?}",
        start.elapsed()
    );
    close(&mut connection)?;
    ensure!(
        connection.disconnect()? == ShutdownStatus::Complete,
        "repeated close after retry changed outcome"
    );
    drop(connection);
    root.finish()?;
    Ok(())
}

/// A watcher panic is reported on every later close and never hides native state.
#[cfg(feature = "fault-injection")]
pub fn watcher_panic_is_sticky() -> Result<()> {
    use wincs::root::FaultPlan;
    let mut root = crate::owned_root::OwnedRoot::new()?;
    let plan = FaultPlan {
        panic_watcher: true,
        ..FaultPlan::default()
    };
    let mut connection = Session::new()
        .with_faults(plan)
        .connect(&root.path, CountingFilter(Arc::default()))?;
    let deadline = Instant::now() + Duration::from_secs(6);
    let first = loop {
        match connection.disconnect() {
            Ok(ShutdownStatus::WatcherStopping | ShutdownStatus::CallbacksActive) => {
                ensure!(
                    Instant::now() < deadline,
                    "panicking watcher never finished"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(ShutdownStatus::Complete) => bail!("watcher panic reported as Complete"),
            Err(error) => break error,
        }
    };
    println!("watcher panic reported: {first}");
    ensure!(!connection.is_connected(), "native disconnect not recorded");
    ensure!(
        connection.disconnect().is_err(),
        "watcher panic was not sticky"
    );
    // Native callbacks really ended: a new connection succeeds.
    let mut next = Session::new().connect(&root.path, CountingFilter(Arc::default()))?;
    close(&mut next)?;
    drop(next);
    drop(connection);
    root.finish()?;
    Ok(())
}

use std::process::ExitCode;

use libtest_mimic::{run, Arguments, Trial};

mod async_filter;
mod failure;
mod handles;
mod lifecycle;
mod owned_root;
mod population;
mod shutdown;
mod sync_filter;
mod usn;

fn main() -> ExitCode {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--expect-read-failure"))
    {
        return if std::fs::read(std::env::args_os().nth(2).expect("owned file")).is_err() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--verify-owned-registrations-absent"))
    {
        if let Err(error) = owned_root::verify_absent(std::path::Path::new(
            &std::env::args_os().nth(2).expect("owned parent"),
        )) {
            eprintln!("{error:#}");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--write-client")) {
        std::fs::write(std::env::args_os().nth(3).unwrap(), b"ready").unwrap();
        std::fs::write(std::env::args_os().nth(2).unwrap(), b"generation N+1").unwrap();
        return ExitCode::SUCCESS;
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--verify-client")) {
        let root = std::env::args().nth(2).expect("owned test root");
        test_list_folders(&root);
        test_read_file(&root);
        return ExitCode::SUCCESS;
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--verify-paged-client"))
    {
        population::verify_client(std::path::Path::new(
            &std::env::args_os().nth(2).expect("root"),
        ))
        .unwrap();
        return ExitCode::SUCCESS;
    }
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--cleanup-owned-failed-root"))
    {
        owned_root::cleanup_failed(std::path::Path::new(
            &std::env::args_os().nth(2).expect("root"),
        ))
        .unwrap();
        return ExitCode::SUCCESS;
    }
    if std::env::var("SAMBAFIED_RUN_CLOUD_FILTER_TESTS").as_deref() != Ok("1") {
        eprintln!("NOT_RUN: Cloud Files registration tests require SAMBAFIED_RUN_CLOUD_FILTER_TESTS=1; see FORK.md");
        return ExitCode::SUCCESS;
    }
    let args = Arguments::from_args();
    #[allow(unused_mut)]
    let mut tests = vec![
        Trial::test("disconnect_during_inflight_callbacks", || {
            lifecycle::disconnect_during_inflight()
                .map_err(|e| libtest_mimic::Failed::from(format!("{e:#}")))
        }),
        Trial::test("failed_recall_completes_pending_read", || {
            failure::test().map_err(|e| libtest_mimic::Failed::from(format!("{e:#}")))
        }),
        Trial::test("fallible_disconnect_repeat_reconnect", || {
            shutdown::test().map_err(|e| libtest_mimic::Failed::from(format!("{e:#}")))
        }),
        Trial::test("handle_method_matrix", || {
            handles::operations().map_err(|e| libtest_mimic::Failed::from(format!("{e:#}")))
        }),
        Trial::test("handle_ownership", || {
            handles::ownership().map_err(|e| libtest_mimic::Failed::from(format!("{e:#}")))
        }),
        Trial::test("acknowledgement_busy_and_mapped", || {
            handles::busy_and_mapped().map_err(|e| libtest_mimic::Failed::from(format!("{e:#}")))
        }),
        Trial::test("protected_oplock_break", || {
            handles::oplock_break().map_err(|e| libtest_mimic::Failed::from(format!("{e:#}")))
        }),
        Trial::test("sync_filter", || {
            sync_filter::test().map_err(|error| libtest_mimic::Failed::from(format!("{error:#}")))
        }),
        Trial::test("async_filter", || {
            async_filter::test().map_err(|error| libtest_mimic::Failed::from(format!("{error:#}")))
        }),
        Trial::test("sync_paged_empty", || {
            population::test(false)
                .map_err(|error| libtest_mimic::Failed::from(format!("{error:#}")))
        }),
        Trial::test("async_paged_empty", || {
            population::test(true)
                .map_err(|error| libtest_mimic::Failed::from(format!("{error:#}")))
        }),
        Trial::test("usn_pointer_output_and_stale_generation", || {
            usn::test().map_err(|error| libtest_mimic::Failed::from(format!("{error:#}")))
        }),
        Trial::test("exclusive_generation_acknowledgement", || {
            usn::exclusive_acceptance()
                .map_err(|error| libtest_mimic::Failed::from(format!("{error:#}")))
        }),
        Trial::test("acknowledgement_competing_writer", || {
            usn::competing_writer()
                .map_err(|error| libtest_mimic::Failed::from(format!("{error:#}")))
        }),
        Trial::test("registration_cleanup_after_unwind", || {
            owned_root::cleanup_after_unwind()
                .map_err(|error| libtest_mimic::Failed::from(format!("{error:#}")))
        }),
    ];
    #[cfg(feature = "fault-injection")]
    tests.extend([
        Trial::test("rollback_after_watcher_spawn_failure", || {
            lifecycle::rollback_after_watcher_spawn_failure()
                .map_err(|e| libtest_mimic::Failed::from(format!("{e:#}")))
        }),
        Trial::test("failed_disconnect_retains_callbacks", || {
            lifecycle::failed_disconnect_retains_callbacks()
                .map_err(|e| libtest_mimic::Failed::from(format!("{e:#}")))
        }),
        Trial::test("watcher_panic_is_sticky", || {
            lifecycle::watcher_panic_is_sticky()
                .map_err(|e| libtest_mimic::Failed::from(format!("{e:#}")))
        }),
    ]);
    run(&args, tests).exit_code()
}

fn test_list_folders(root: &str) {
    fn collect(root: &std::path::Path, relative: &std::path::Path, names: &mut Vec<String>) {
        for entry in std::fs::read_dir(root.join(relative)).unwrap() {
            let entry = entry.unwrap();
            let next = relative.join(entry.file_name());
            names.push(next.to_string_lossy().into_owned());
            if entry.file_type().unwrap().is_dir() {
                collect(root, &next, names);
            }
        }
    }
    let mut names = Vec::new();
    collect(
        std::path::Path::new(root),
        std::path::Path::new(""),
        &mut names,
    );
    names.sort();
    assert_eq!(names, ["dir1", "dir1\\test2.txt", "test1.txt"]);
}

fn test_read_file(root: &str) {
    for relative in ["test1.txt", "dir1\\test2.txt"] {
        let content = std::fs::read_to_string(std::path::Path::new(root).join(relative)).unwrap();
        assert_eq!(content, relative);
    }
}

fn run_client(root: &str) -> anyhow::Result<()> {
    run_client_mode(std::path::Path::new(root), "--verify-client")
}

fn run_client_mode(root: &std::path::Path, mode: &str) -> anyhow::Result<()> {
    let mut child = std::process::Command::new(std::env::current_exe()?)
        .arg(mode)
        .arg(root)
        .spawn()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        if let Some(status) = child.try_wait()? {
            anyhow::ensure!(status.success(), "external client failed: {status}");
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            child.kill()?;
            child.wait()?;
            anyhow::bail!("external native client timed out");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

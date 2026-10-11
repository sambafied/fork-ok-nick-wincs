use std::process::ExitCode;

use libtest_mimic::{run, Arguments, Trial};

mod async_filter;
mod owned_root;
mod population;
mod sync_filter;
mod usn;

fn main() -> ExitCode {
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
    let tests = vec![
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
        Trial::test("current_nonzero_usn_acceptance", || {
            usn::current_nonzero_acceptance()
                .map_err(|error| libtest_mimic::Failed::from(format!("{error:#}")))
        })
        .with_ignored_flag(std::env::var("SAMBAFIED_QUALIFY_CURRENT_USN").as_deref() != Ok("1")),
        Trial::test("registration_cleanup_after_unwind", || {
            owned_root::cleanup_after_unwind()
                .map_err(|error| libtest_mimic::Failed::from(format!("{error:#}")))
        }),
    ];
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

use std::process::ExitCode;

use libtest_mimic::{run, Arguments, Trial};

mod async_filter;
mod sync_filter;

fn main() -> ExitCode {
    if std::env::var("SAMBAFIED_RUN_CLOUD_FILTER_TESTS").as_deref() != Ok("1") {
        eprintln!("NOT_RUN: Cloud Files registration tests require SAMBAFIED_RUN_CLOUD_FILTER_TESTS=1; see FORK.md");
        return ExitCode::SUCCESS;
    }
    let args = Arguments::from_args();
    let tests = vec![Trial::test("sync_filter", sync_filter::test)];

    let conclusion = run(&args, tests);
    if conclusion.has_failed() {
        return conclusion.exit_code();
    }

    let tests = vec![Trial::test("async_filter", async_filter::test)];
    let conclusion = run(&args, tests);

    conclusion.exit_code()
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

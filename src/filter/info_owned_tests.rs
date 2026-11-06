use super::*;
use crate::filter::metadata::MAX_WIDE_UNITS;
use windows::core::PCWSTR;

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

#[cfg(not(feature = "globs"))]
fn assert_pattern(info: &FetchPlaceholders, expected: &str) {
    assert_eq!(info.pattern(), expected);
}

#[cfg(feature = "globs")]
fn assert_pattern(info: &FetchPlaceholders, expected: &str) {
    assert_eq!(
        info.pattern().expect("valid captured glob").glob(),
        expected
    );
}

#[test]
fn metadata_survives_mutation_free_and_thread_transfer_before_first_getter() {
    let mut pattern = wide("private-资料-*.txt");
    let mut target = wide(r"new\目录😀.txt");
    let mut source = wide(r"old\資料😀.txt");
    let mut native_volume = wide(r"Q:\private-root");
    let fetch_flags = CloudFilters::CF_CALLBACK_FETCH_PLACEHOLDERS_FLAGS(0x40);
    let rename_flags = CloudFilters::CF_CALLBACK_RENAME_FLAG_IS_DIRECTORY
        | CloudFilters::CF_CALLBACK_RENAME_FLAG_SOURCE_IN_SCOPE
        | CloudFilters::CF_CALLBACK_RENAME_FLAG_TARGET_IN_SCOPE;
    let renamed_flags = CloudFilters::CF_CALLBACK_RENAME_COMPLETION_FLAGS(0x20);

    let fetch = unsafe {
        FetchPlaceholders::new(CF_CALLBACK_PARAMETERS_0_3 {
            Flags: fetch_flags,
            Pattern: PCWSTR(pattern.as_ptr()),
        })
    }
    .expect("capture pattern");
    let rename = unsafe {
        Rename::new(
            CF_CALLBACK_PARAMETERS_0_10 {
                Flags: rename_flags,
                TargetPath: PCWSTR(target.as_ptr()),
            },
            &native_volume[..native_volume.len() - 1],
        )
    }
    .expect("capture target");
    let renamed = unsafe {
        Renamed::new(
            CF_CALLBACK_PARAMETERS_0_11 {
                Flags: renamed_flags,
                SourcePath: PCWSTR(source.as_ptr()),
            },
            &native_volume[..native_volume.len() - 1],
        )
    }
    .expect("capture source");

    pattern.fill(b'x' as u16);
    target.fill(b'x' as u16);
    source.fill(b'x' as u16);
    native_volume.fill(b'x' as u16);
    drop((pattern, target, source, native_volume));

    std::thread::spawn(move || {
        assert_pattern(&fetch, "private-资料-*.txt");
        assert_eq!(fetch.flags, fetch_flags);
        assert!(rename.is_directory());
        assert!(rename.source_in_scope());
        assert!(rename.target_in_scope());
        assert_eq!(rename.flags, rename_flags);
        assert_eq!(
            rename.target_path(),
            PathBuf::from(r"Q:\private-root\new\目录😀.txt")
        );
        assert_eq!(renamed.flags, renamed_flags);
        assert_eq!(
            renamed.source_path(),
            PathBuf::from(r"Q:\private-root\old\資料😀.txt")
        );
        assert_eq!(format!("{fetch:?}"), "FetchPlaceholders { .. }");
        assert_eq!(format!("{renamed:?}"), "Renamed { .. }");
        let debug = format!("{rename:?}");
        assert!(!debug.contains("private-root"));
        assert!(!debug.contains("目录"));
        assert!(!debug.contains("target_path"));
    })
    .join()
    .expect("owned metadata works on another thread");
}

#[test]
fn optional_null_pattern_is_unrestricted_and_empty_remains_empty() {
    let null = unsafe { FetchPlaceholders::new(CF_CALLBACK_PARAMETERS_0_3::default()) }
        .expect("optional null pattern");
    let mut empty = vec![0u16];
    let captured_empty = unsafe {
        FetchPlaceholders::new(CF_CALLBACK_PARAMETERS_0_3 {
            Pattern: PCWSTR(empty.as_ptr()),
            ..Default::default()
        })
    }
    .expect("empty pattern");
    empty.fill(b'x' as u16);
    drop(empty);
    std::thread::spawn(move || {
        assert_pattern(&null, "*");
        assert_pattern(&captured_empty, "");
    })
    .join()
    .unwrap();
}

#[test]
fn required_null_paths_are_rejected() {
    assert!(matches!(
        unsafe { Rename::new(CF_CALLBACK_PARAMETERS_0_10::default(), &wide("Q:")[..2]) },
        Err(MetadataError::InvalidString)
    ));
    assert!(matches!(
        unsafe { Renamed::new(CF_CALLBACK_PARAMETERS_0_11::default(), &wide("Q:")[..2]) },
        Err(MetadataError::InvalidString)
    ));
}

#[test]
fn each_rename_flag_is_preserved_independently() {
    for (flags, directory, source, target) in [
        (
            CloudFilters::CF_CALLBACK_RENAME_FLAGS(0),
            false,
            false,
            false,
        ),
        (
            CloudFilters::CF_CALLBACK_RENAME_FLAG_IS_DIRECTORY,
            true,
            false,
            false,
        ),
        (
            CloudFilters::CF_CALLBACK_RENAME_FLAG_SOURCE_IN_SCOPE,
            false,
            true,
            false,
        ),
        (
            CloudFilters::CF_CALLBACK_RENAME_FLAG_TARGET_IN_SCOPE,
            false,
            false,
            true,
        ),
    ] {
        let mut path = wide("child");
        let info = unsafe {
            Rename::new(
                CF_CALLBACK_PARAMETERS_0_10 {
                    Flags: flags,
                    TargetPath: PCWSTR(path.as_ptr()),
                },
                &wide("Q:")[..2],
            )
        }
        .unwrap();
        path.fill(b'x' as u16);
        drop(path);
        std::thread::spawn(move || {
            assert_eq!(info.is_directory(), directory);
            assert_eq!(info.source_in_scope(), source);
            assert_eq!(info.target_in_scope(), target);
        })
        .join()
        .unwrap();
    }
}

#[test]
fn maximum_terminated_buffers_are_owned() {
    let mut buffer = vec![b'a' as u16; MAX_WIDE_UNITS];
    buffer[MAX_WIDE_UNITS - 1] = 0;
    let fetch = unsafe {
        FetchPlaceholders::new(CF_CALLBACK_PARAMETERS_0_3 {
            Pattern: PCWSTR(buffer.as_ptr()),
            ..Default::default()
        })
    }
    .unwrap();
    let rename = unsafe {
        Rename::new(
            CF_CALLBACK_PARAMETERS_0_10 {
                TargetPath: PCWSTR(buffer.as_ptr()),
                ..Default::default()
            },
            &[],
        )
    }
    .unwrap();
    let renamed = unsafe {
        Renamed::new(
            CF_CALLBACK_PARAMETERS_0_11 {
                SourcePath: PCWSTR(buffer.as_ptr()),
                ..Default::default()
            },
            &[],
        )
    }
    .unwrap();
    buffer.fill(b'x' as u16);
    drop(buffer);
    std::thread::spawn(move || {
        let expected = "a".repeat(MAX_WIDE_UNITS - 1);
        assert_pattern(&fetch, &expected);
        assert_eq!(rename.target_path(), PathBuf::from(&expected));
        assert_eq!(renamed.source_path(), PathBuf::from(&expected));
    })
    .join()
    .unwrap();
}

#[test]
fn unterminated_readable_maximum_buffers_are_rejected() {
    // This allocation is readable across the full allowed scan. No forged
    // pointer or shorter allocation is passed to the unsafe constructors.
    let buffer = vec![b'a' as u16; MAX_WIDE_UNITS];
    assert!(matches!(
        unsafe {
            FetchPlaceholders::new(CF_CALLBACK_PARAMETERS_0_3 {
                Pattern: PCWSTR(buffer.as_ptr()),
                ..Default::default()
            })
        },
        Err(MetadataError::InvalidString)
    ));
    assert!(matches!(
        unsafe {
            Rename::new(
                CF_CALLBACK_PARAMETERS_0_10 {
                    TargetPath: PCWSTR(buffer.as_ptr()),
                    ..Default::default()
                },
                &[],
            )
        },
        Err(MetadataError::InvalidString)
    ));
    assert!(matches!(
        unsafe {
            Renamed::new(
                CF_CALLBACK_PARAMETERS_0_11 {
                    SourcePath: PCWSTR(buffer.as_ptr()),
                    ..Default::default()
                },
                &[],
            )
        },
        Err(MetadataError::InvalidString)
    ));
}

#[cfg(feature = "globs")]
#[test]
fn invalid_glob_remains_an_accessor_error_after_native_buffer_is_freed() {
    let mut buffer = wide("[");
    let info = unsafe {
        FetchPlaceholders::new(CF_CALLBACK_PARAMETERS_0_3 {
            Pattern: PCWSTR(buffer.as_ptr()),
            ..Default::default()
        })
    }
    .unwrap();
    buffer.fill(b'x' as u16);
    drop(buffer);
    std::thread::spawn(move || {
        assert!(info.pattern().is_err());
        assert_eq!(format!("{info:?}"), "FetchPlaceholders { .. }");
    })
    .join()
    .unwrap();
}

#[test]
fn rename_volume_bounds_are_checked_before_native_path_capture() {
    // Null native paths are intentional: oversized volume must refuse before
    // attempting path capture. Empty volume remains supported by other tests.
    let volume = vec![b'v' as u16; MAX_WIDE_UNITS];
    assert!(matches!(
        unsafe { Rename::new(CF_CALLBACK_PARAMETERS_0_10::default(), &volume) },
        Err(MetadataError::InvalidString)
    ));
    assert!(matches!(
        unsafe { Renamed::new(CF_CALLBACK_PARAMETERS_0_11::default(), &volume) },
        Err(MetadataError::InvalidString)
    ));
}

#[test]
fn maximum_volume_is_owned_before_backing_mutation_and_free() {
    let mut volume = vec![b'v' as u16; MAX_WIDE_UNITS - 1];
    let path = wide("child");
    let rename = unsafe {
        Rename::new(
            CF_CALLBACK_PARAMETERS_0_10 {
                TargetPath: PCWSTR(path.as_ptr()),
                ..Default::default()
            },
            &volume,
        )
    }
    .unwrap();
    let renamed = unsafe {
        Renamed::new(
            CF_CALLBACK_PARAMETERS_0_11 {
                SourcePath: PCWSTR(path.as_ptr()),
                ..Default::default()
            },
            &volume,
        )
    }
    .unwrap();
    volume.fill(b'x' as u16);
    drop((volume, path));
    std::thread::spawn(move || {
        let mut expected = PathBuf::from("v".repeat(MAX_WIDE_UNITS - 1));
        expected.push("child");
        assert_eq!(rename.target_path(), expected);
        assert_eq!(renamed.source_path(), expected);
    })
    .join()
    .unwrap();
}

use anyhow::{ensure, Context, Result};
use std::{
    fs,
    mem::{size_of, size_of_val},
    os::windows::io::AsRawHandle,
    path::Path,
};
use wincs::placeholder::{ConvertOptions, Placeholder};
use windows::Win32::{
    Foundation::HANDLE,
    System::{
        Ioctl::{FSCTL_READ_FILE_USN_DATA, READ_FILE_USN_DATA, USN_RECORD_V2},
        IO::DeviceIoControl,
    },
};

fn file_usn(path: &Path) -> Result<i64> {
    let file = fs::File::open(path)?;
    usn_for_handle(HANDLE(file.as_raw_handle()))
}

fn usn_for_handle(handle: HANDLE) -> Result<i64> {
    let input = READ_FILE_USN_DATA {
        MinMajorVersion: 2,
        MaxMajorVersion: 2,
    };
    let mut buffer = [0u64; 512];
    let mut returned = 0;
    unsafe {
        DeviceIoControl(
            handle,
            FSCTL_READ_FILE_USN_DATA,
            Some((&input as *const READ_FILE_USN_DATA).cast()),
            size_of::<READ_FILE_USN_DATA>() as u32,
            Some(buffer.as_mut_ptr().cast()),
            size_of_val(&buffer) as u32,
            Some(&mut returned),
            None,
        )
    }?;
    ensure!(
        returned as usize >= size_of::<USN_RECORD_V2>(),
        "short USN record"
    );
    let record = unsafe { &*(buffer.as_ptr().cast::<USN_RECORD_V2>()) };
    ensure!(record.MajorVersion == 2, "unexpected USN version");
    ensure!(record.Usn != 0, "actual file USN must be nonzero");
    Ok(record.Usn)
}

pub fn test() -> anyhow::Result<()> {
    run(false)
}
pub fn current_nonzero_acceptance() -> anyhow::Result<()> {
    run(true)
}

fn run(require_current_acceptance: bool) -> anyhow::Result<()> {
    let mut owned = crate::owned_root::OwnedRoot::new()?;
    let connection =
        wincs::root::Session::new().connect(&owned.path, crate::owned_root::EmptyFilter)?;
    let path = owned.path.join("usn.txt");
    fs::write(&path, b"generation N").context("write initial fixture")?;
    let mut placeholder = Placeholder::options()
        .write_access()
        .foreground()
        .open(&path)?;
    // A nonzero sentinel would be an invalid address with the former integer cast.
    // ConvertUsn is output-only, so the sentinel must never be dereferenced.
    let mut converted = 17i64;
    placeholder
        .convert_to_placeholder(
            ConvertOptions::default().blob(b"owned-usn".to_vec()),
            Some(&mut converted),
        )
        .context("convert ordinary to placeholder with nonzero output sentinel")?;
    ensure!(converted != 17, "conversion did not write output USN");
    drop(placeholder);
    let mut placeholder =
        Placeholder::from(fs::OpenOptions::new().read(true).write(true).open(&path)?);
    // Read the USN through the same handle: closing/reopening an observer can
    // itself advance this cloud file's USN before the conditional operation.
    let current = usn_for_handle(placeholder.inner_handle().handle())?;
    println!("before matching state: convert output={converted}, same-handle current={current}");
    let mut matching = current;
    let wrapper_result = placeholder
        .mark_in_sync(true, Some(&mut matching))
        .map(|_| ());
    if let Err(error) = &wrapper_result {
        let mut control_usn = usn_for_handle(placeholder.inner_handle().handle())?;
        let control = unsafe {
            windows::Win32::Storage::CloudFilters::CfSetInSyncState(
                placeholder.inner_handle().handle(),
                windows::Win32::Storage::CloudFilters::CF_IN_SYNC_STATE_IN_SYNC,
                windows::Win32::Storage::CloudFilters::CF_SET_IN_SYNC_FLAG_NONE,
                Some(std::ptr::from_mut(&mut control_usn)),
            )
        };
        println!("matching USN wrapper={error}; direct native control={control:?}; control USN={control_usn}");
        ensure!(
            error.code().0 as u32 == 0x80070179,
            "unexpected wrapper failure"
        );
        ensure!(
            control
                .err()
                .context("direct native control unexpectedly succeeded")?
                .code()
                == error.code(),
            "wrapper differs from native control"
        );
    }
    if require_current_acceptance {
        wrapper_result.context("mark matching nonzero USN in sync; unresolved fork #15")?;
    } else if wrapper_result.is_err() {
        println!("LIMITATION fork #15: current nonzero USN acceptance NOT_PASSED; direct native control reproduces rejection");
        // Establish a test baseline only. Unconditional marking is NOT a
        // production substitute for expected-generation/USN concurrency safety.
        placeholder.mark_in_sync(true, None)?;
    }
    ensure!(
        placeholder
            .info()
            .context("query converted info")?
            .context("placeholder info")?
            .is_in_sync(),
        "matching USN not in sync"
    );
    drop(placeholder);
    let old = file_usn(&path)?;
    fs::write(&path, b"generation N+1")?;
    fs::OpenOptions::new().write(true).open(&path)?.sync_all()?;
    let changed = file_usn(&path)?;
    ensure!(old != changed, "edit did not advance real USN");
    let mut placeholder = Placeholder::options()
        .write_access()
        .foreground()
        .open(&path)?;
    // Explicitly mark dirty even if the chosen registration has no auto-clear policy.
    placeholder.mark_in_sync(false, None)?;
    let mut stale = old;
    let error = placeholder
        .mark_in_sync(true, Some(&mut stale))
        .err()
        .context("stale USN accepted")?;
    ensure!(
        error.code().0 as u32 == 0x80070179,
        "unexpected stale USN error: {error}"
    );
    ensure!(
        !placeholder.info()?.context("dirty info")?.is_in_sync(),
        "later generation marked clean"
    );
    ensure!(fs::read(&path)? == b"generation N+1", "later bytes lost");
    println!("USN conversion output={converted}, current input={current}, current output={matching}; stale={old}, changed={changed}, rejection={error}");
    drop(placeholder);
    drop(connection);
    owned.finish()?;
    ensure!(
        fs::read(&path)? == b"generation N+1",
        "unregister lost later bytes"
    );
    Ok(())
}

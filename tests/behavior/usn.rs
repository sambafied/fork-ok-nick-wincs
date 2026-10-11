use anyhow::{ensure, Context, Result};
use std::{fs, path::Path};
use wincs::placeholder::{ConvertOptions, Placeholder, UploadAcknowledgement};
use windows::Win32::Storage::CloudFilters::{
    CfSetInSyncState, CF_IN_SYNC_STATE_IN_SYNC, CF_SET_IN_SYNC_FLAG_NONE,
};

fn observe(path: &Path) -> Result<wincs::placeholder::FileGeneration> {
    let placeholder = Placeholder::from(fs::File::open(path)?);
    Ok(placeholder.generation()?)
}

fn fixture(path: &Path) -> Result<()> {
    fs::write(path, b"generation N")?;
    let mut placeholder = Placeholder::options()
        .write_access()
        .foreground()
        .open(path)?;
    let mut output = 17;
    placeholder.convert_to_placeholder(
        ConvertOptions::default().blob(b"owned-usn".to_vec()),
        Some(&mut output),
    )?;
    ensure!(output != 17, "conversion did not write output");
    println!("conversion output={output}; output zero is not a concurrency token");
    Ok(())
}

pub fn test() -> Result<()> {
    let mut owned = crate::owned_root::OwnedRoot::new()?;
    let connection =
        wincs::root::Session::new().connect(&owned.path, crate::owned_root::EmptyFilter)?;
    let path = owned.path.join("usn.txt");
    fixture(&path)?;
    let generation = observe(&path)?;
    ensure!(generation.usn() != 0);
    // Characterize raw CFAPI. This is not the chosen production strategy, nor
    // counted as positive raw-USN acceptance if the native API rejects it.
    {
        let mut placeholder = Placeholder::options()
            .write_access()
            .exclusive()
            .open(&path)?;
        let reference = placeholder.win32_handle()?;
        let mut token = placeholder.generation()?.usn();
        let wrapper = placeholder.mark_in_sync(true, Some(&mut token)).map(|_| ());
        if let Err(error) = &wrapper {
            let mut native_token = placeholder.generation()?.usn();
            let native = unsafe {
                CfSetInSyncState(
                    reference.handle(),
                    CF_IN_SYNC_STATE_IN_SYNC,
                    CF_SET_IN_SYNC_FLAG_NONE,
                    Some(std::ptr::from_mut(&mut native_token)),
                )
            };
            ensure!(error.code().0 as u32 == 0x80070179);
            ensure!(
                native
                    .err()
                    .context("raw control unexpectedly succeeded")?
                    .code()
                    == error.code()
            );
            println!(
                "RAW_USN_UNSUPPORTED dirty-to-clean: wrapper={error}; direct control same error"
            );
        } else {
            println!("RAW_USN_ACCEPTED on this host");
        }
    }
    // Re-observe after the diagnostic: even failed native operations can advance
    // the file's journal USN. Never retry a prior token as an unconditional call.
    let old = observe(&path)?;
    fs::write(&path, b"generation N+1")?;
    let current = observe(&path)?;
    ensure!(old != current, "edit failed to change USN");
    ensure!(
        Placeholder::acknowledge_uploaded(&path, old).is_err(),
        "stale generation accepted"
    );
    ensure!(
        !Placeholder::open(&path)?
            .info()?
            .context("info")?
            .is_in_sync(),
        "later generation clean"
    );
    ensure!(fs::read(&path)? == b"generation N+1");
    println!("stale identity/USN rejected, later generation remains dirty");
    drop(connection);
    owned.finish()?;
    ensure!(fs::read(&path)? == b"generation N+1");
    Ok(())
}

pub fn exclusive_acceptance() -> Result<()> {
    let mut owned = crate::owned_root::OwnedRoot::new()?;
    let connection =
        wincs::root::Session::new().connect(&owned.path, crate::owned_root::EmptyFilter)?;
    let path = owned.path.join("ack.txt");
    fixture(&path)?;
    let expected = observe(&path)?;
    let acknowledged = Placeholder::acknowledge_uploaded(&path, expected)?;
    ensure!(acknowledged.usn() != 0);
    ensure!(Placeholder::open(&path)?
        .info()?
        .context("info")?
        .is_in_sync());
    ensure!(fs::read(&path)? == b"generation N");
    println!(
        "exclusive acknowledgement PASS: expected {expected:?}; acknowledged {acknowledged:?}"
    );
    // Same name, different file identity must never inherit the old upload.
    fs::rename(&path, owned.path.join("old.txt"))?;
    fixture(&path)?;
    ensure!(
        Placeholder::acknowledge_uploaded(&path, expected).is_err(),
        "replacement accepted"
    );
    ensure!(!Placeholder::open(&path)?
        .info()?
        .context("info")?
        .is_in_sync());
    println!("replacement identity rejected");
    drop(connection);
    owned.finish()?;
    Ok(())
}

pub fn competing_writer() -> Result<()> {
    let mut owned = crate::owned_root::OwnedRoot::new()?;
    let connection =
        wincs::root::Session::new().connect(&owned.path, crate::owned_root::EmptyFilter)?;
    let path = owned.path.join("ack.txt");
    fixture(&path)?;
    let expected = observe(&path)?;
    let barrier = UploadAcknowledgement::begin(&path, expected)?;
    let marker = owned.path.join("writer-ready");
    let mut child = std::process::Command::new(std::env::current_exe()?)
        .arg("--write-client")
        .arg(&path)
        .arg(&marker)
        .spawn()?;
    let result = (|| -> Result<()> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        ensure!(marker.exists(), "writer did not reach open");
        std::thread::sleep(std::time::Duration::from_millis(100));
        ensure!(child.try_wait()?.is_none(), "writer bypassed barrier");
        barrier.commit()?;
        while child.try_wait()?.is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        ensure!(child.try_wait()?.context("writer timeout")?.success());
        ensure!(fs::read(&path)? == b"generation N+1");
        ensure!(
            !Placeholder::open(&path)?
                .info()?
                .context("info")?
                .is_in_sync(),
            "N+1 stays clean"
        );
        println!("competing writer waited for barrier, N+1 then dirtied and survived");
        Ok(())
    })();
    // Even assertion/ordinary errors must not strand a blocked child.
    if child.try_wait()?.is_none() {
        child.kill()?;
        child.wait()?;
    }
    result?;
    drop(connection);
    owned.finish()?;
    ensure!(fs::read(&path)? == b"generation N+1");
    Ok(())
}

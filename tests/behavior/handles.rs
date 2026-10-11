use anyhow::{ensure, Context, Result};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    os::windows::io::AsRawHandle,
};
use wincs::placeholder::{
    ConvertOptions, PinOptions, PinState, Placeholder, ReadType, UpdateOptions,
    UploadAcknowledgement,
};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{
            Memory::{
                CreateFileMappingW, MapViewOfFile, UnmapViewOfFile, FILE_MAP_WRITE, PAGE_READWRITE,
            },
            Threading::{GetCurrentProcess, GetProcessHandleCount},
        },
    },
};

fn count() -> Result<u32> {
    let mut n = 0;
    unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut n) }?;
    Ok(n)
}

pub fn operations() -> Result<()> {
    let mut owned = crate::owned_root::OwnedRoot::new()?;
    let connection =
        wincs::root::Session::new().connect(&owned.path, crate::owned_root::EmptyFilter)?;
    for protected in [false, true] {
        let path = owned.path.join(format!("operations-{protected}.txt"));
        let bytes = vec![0x42; 8192];
        fs::write(&path, &bytes)?;
        let mut p = if protected {
            Placeholder::options()
                .write_access()
                .foreground()
                .open(&path)?
        } else {
            Placeholder::from(fs::OpenOptions::new().read(true).write(true).open(&path)?)
        };
        ensure!(
            p.info()?.is_none(),
            "ordinary file identified as placeholder"
        );
        p.convert_to_placeholder(ConvertOptions::default().blob(vec![0x41; 128]), None)?;
        let info = p.info()?.context("converted info")?;
        let clone = info.clone();
        drop(info);
        ensure!(
            clone.blob() == [0x41; 128],
            "identity clone/header offset incorrect"
        );
        ensure!(
            p.fixed_size_info(0).is_err(),
            "undersized native query accepted"
        );
        ensure!(p.fixed_size_info(128)?.context("sized info")?.blob() == [0x41; 128]);
        p.mark_pin(PinState::Unspecified, PinOptions::default())?;
        ensure!(p.info()?.context("pin info")?.pin_state() == PinState::Unspecified);
        p.update(
            UpdateOptions::default()
                .metadata(wincs::metadata::Metadata::file().size(bytes.len() as u64))
                .blob(b"updated"),
            None,
        )?;
        ensure!(p.info()?.context("updated info")?.blob() == b"updated");
        p.hydrate(4096..8192)?;
        let mut ranges = [0u8; 256];
        let returned = p.retrieve_data(ReadType::Any, 0, &mut ranges)?;
        ensure!(
            returned >= 16 && returned as usize <= ranges.len(),
            "range output"
        );
        ensure!(i64::from_ne_bytes(ranges[..8].try_into()?) == 0);
        // Nonempty range invalidates bytes. An empty identity-only update must
        // preserve metadata/data; that also exercises the previously inverted
        // DehydrateRangeArray option and dangling metadata pointer safely.
        ensure!(p
            .hydrate((std::ops::Bound::Included(9), std::ops::Bound::Excluded(5)))
            .is_err());
        p.mark_in_sync(false, None)?;
        drop(p);
        ensure!(fs::read(&path)? == bytes, "operation lost data");
        // Range invalidation requires exclusive access; test both kinds without
        // ever dehydrating through an unprotected share-all handle.
        println!("opening exclusive range handle: protected={protected}");
        let mut exclusive = if protected {
            Placeholder::options()
                .exclusive()
                .write_access()
                .open(&path)?
        } else {
            use std::os::windows::fs::OpenOptionsExt;
            Placeholder::from(
                fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .share_mode(0)
                    .open(&path)?,
            )
        };
        println!("range handle open, marking fixture in sync");
        exclusive.mark_in_sync(true, None)?; // generated test fixture baseline
        println!("issuing nonempty range update");
        exclusive.update(
            UpdateOptions::default().dehydrate_ranges(std::iter::once(0..4096)),
            None,
        )?;
        println!("range update returned, querying ranges");
        let mut ranges = [0u8; 256];
        ensure!(
            exclusive.retrieve_data(ReadType::Any, 0, &mut ranges)? == 0,
            "nonempty dehydration range was omitted"
        );
        ensure!(
            exclusive.retrieve_data(ReadType::Any, 4096, &mut ranges)? >= 16,
            "unaffected range lost"
        );
        ensure!(
            i64::from_ne_bytes(ranges[..8].try_into()?) == 4096,
            "unaffected range offset"
        );
        // Full hydration policy recalls the entire file on subsequent ordinary
        // reads. This provider has no remote binary for that recall, so assert
        // native residency directly instead of creating a spurious timeout.

        println!("range queries returned, closing exclusive");
        drop(exclusive);

        println!("ordinary/protected method matrix PASS: protected={protected}");
    }
    drop(connection);
    owned.finish()?;
    Ok(())
}

pub fn ownership() -> Result<()> {
    let mut owned = crate::owned_root::OwnedRoot::new()?;
    let connection =
        wincs::root::Session::new().connect(&owned.path, crate::owned_root::EmptyFilter)?;
    let path = owned.path.join("ownership.txt");
    fs::write(&path, b"retained")?;
    let mut p = Placeholder::options()
        .write_access()
        .foreground()
        .open(&path)?;
    p.convert_to_placeholder(ConvertOptions::default().blob(b"identity".to_vec()), None)?;
    let reference = p.win32_handle()?;
    let clone = reference.clone();
    drop(p);
    drop(reference);
    // The final guard owns both native reference and opaque owner even after p
    // is dropped. It can still be used for real handle-based APIs.
    let mut id = windows::Win32::Storage::FileSystem::FILE_ID_INFO::default();
    unsafe {
        windows::Win32::Storage::FileSystem::GetFileInformationByHandleEx(
            clone.handle(),
            windows::Win32::Storage::FileSystem::FileIdInfo,
            std::ptr::from_mut(&mut id).cast(),
            std::mem::size_of_val(&id) as u32,
        )
    }?;
    drop(clone);
    let before = count()?;
    for _ in 0..128 {
        let p = Placeholder::options().foreground().open(&path)?;
        let reference = p.win32_handle()?;
        let copy = reference.clone();
        drop(p);
        drop(reference);
        drop(copy);
        let file = fs::OpenOptions::new().read(true).write(true).open(&path)?;
        let mut file: fs::File = Placeholder::from(file).try_into()?;
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        ensure!(bytes == b"retained");
    }
    let after = count()?;
    ensure!(after <= before + 2, "handle leak: {before} -> {after}");
    let protected = Placeholder::options().foreground().open(&path)?;
    let result: windows::core::Result<fs::File> = protected.try_into();
    ensure!(
        result
            .err()
            .context("unsafe protected File conversion accepted")?
            .code()
            == windows::Win32::Foundation::E_INVALIDARG
    );
    ensure!(
        Placeholder::open(&path)?
            .info()?
            .context("still placeholder")?
            .blob()
            == b"identity",
        "conversion reverted metadata"
    );
    println!("reference clones/owner drop and 128 File transfers PASS; handles {before}->{after}; protected File rejected without revert");
    drop(connection);
    owned.finish()?;
    ensure!(fs::read(&path)? == b"retained");
    Ok(())
}

struct Mapping {
    handle: HANDLE,
    view: windows::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS,
}
impl Drop for Mapping {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.handle);
        }
    }
}

pub fn busy_and_mapped() -> Result<()> {
    let mut owned = crate::owned_root::OwnedRoot::new()?;
    let connection =
        wincs::root::Session::new().connect(&owned.path, crate::owned_root::EmptyFilter)?;
    let path = owned.path.join("mapped.txt");
    fs::write(&path, b"generation N")?;
    let mut p = Placeholder::options()
        .write_access()
        .foreground()
        .open(&path)?;
    p.convert_to_placeholder(ConvertOptions::default().blob(b"mapped".to_vec()), None)?;
    drop(p);
    let expected = Placeholder::from(fs::File::open(&path)?).generation()?;
    let file = fs::OpenOptions::new().read(true).write(true).open(&path)?;
    ensure!(
        UploadAcknowledgement::begin(&path, expected).is_err(),
        "existing writer allowed acknowledgement"
    );
    let handle = unsafe {
        CreateFileMappingW(
            HANDLE(file.as_raw_handle()),
            None,
            PAGE_READWRITE,
            0,
            0,
            PCWSTR::null(),
        )
    }?;
    let view = unsafe { MapViewOfFile(handle, FILE_MAP_WRITE, 0, 0, 0) };
    if view.Value.is_null() {
        unsafe { CloseHandle(handle) }?;
        anyhow::bail!("map view failed");
    }
    let mapping = Mapping { handle, view };
    drop(file); // writable section remains even though there are no open files
    let rejection = UploadAcknowledgement::begin(&path, expected)
        .err()
        .context("writable mapping allowed acknowledgement")?;
    unsafe {
        std::ptr::copy_nonoverlapping(
            b"generation M".as_ptr(),
            mapping.view.Value.cast::<u8>(),
            12,
        );
    }
    unsafe { windows::Win32::System::Memory::FlushViewOfFile(mapping.view.Value, 12) }?;
    drop(mapping);
    ensure!(fs::read(&path)? == b"generation M");
    ensure!(!Placeholder::open(&path)?
        .info()?
        .context("mapped info")?
        .is_in_sync());
    println!("open writer and surviving writable view reject barrier; mapped bytes retained: {rejection}");
    drop(connection);
    owned.finish()?;
    ensure!(fs::read(&path)? == b"generation M");
    Ok(())
}

pub fn oplock_break() -> Result<()> {
    let mut owned = crate::owned_root::OwnedRoot::new()?;
    let connection =
        wincs::root::Session::new().connect(&owned.path, crate::owned_root::EmptyFilter)?;
    let path = owned.path.join("break.txt");
    fs::write(&path, b"generation N")?;
    let p = Placeholder::options()
        .write_access()
        .exclusive()
        .open(&path)?;
    let reference = p.win32_handle()?;
    let clone = reference.clone();
    let marker = owned.path.join("break-ready");
    let mut child = std::process::Command::new(std::env::current_exe()?)
        .arg("--write-client")
        .arg(&path)
        .arg(&marker)
        .spawn()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !marker.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let ready = marker.exists();
    std::thread::sleep(std::time::Duration::from_millis(100));
    let held = child.try_wait()?.is_none();
    drop(reference);
    let clone_held = child.try_wait()?.is_none();
    drop(clone); // final clone releases break acknowledgement
    while child.try_wait()?.is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let status = child.try_wait()?;
    if status.is_none() {
        child.kill()?;
        child.wait()?;
    }
    ensure!(
        ready && held && clone_held,
        "reference did not hold conflicting open"
    );
    ensure!(status.context("writer timeout")?.success());
    let before = count()?;
    for _ in 0..128 {
        ensure!(
            p.win32_handle()
                .err()
                .context("reference succeeded after break")?
                .code()
                == windows::Win32::Foundation::E_HANDLE
        );
    }
    ensure!(
        count()? <= before + 2,
        "failed reference acquisition leaked"
    );
    drop(p);
    ensure!(fs::read(&path)? == b"generation N+1");
    println!("oplock break delayed through last clone; 128 failed acquisitions return E_HANDLE without leak");
    drop(connection);
    owned.finish()?;
    Ok(())
}

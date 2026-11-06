use super::super::metadata::MAX_WIDE_UNITS;
use super::*;
use windows::core::PCWSTR;

const GUID: &[u16] = &[71, 85, 73, 68, 0];
const DOS: &[u16] = &[67, 58, 0];
const NORMALIZED: &[u16] = &[100, 105, 114, 0];

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn process(id: u32, session_id: u32) -> CF_PROCESS_INFO {
    let mut value: CF_PROCESS_INFO = unsafe { std::mem::zeroed() };
    value.StructSize = std::mem::size_of::<CF_PROCESS_INFO>() as u32;
    value.ProcessId = id;
    value.SessionId = session_id;
    value
}

fn callback(process: *const CF_PROCESS_INFO) -> CF_CALLBACK_INFO {
    let mut value: CF_CALLBACK_INFO = unsafe { std::mem::zeroed() };
    value.StructSize = std::mem::size_of::<CF_CALLBACK_INFO>() as u32;
    value.VolumeGuidName = PCWSTR(GUID.as_ptr());
    value.VolumeDosName = PCWSTR(DOS.as_ptr());
    value.NormalizedPath = PCWSTR(NORMALIZED.as_ptr());
    value.ProcessInfo = process as _;
    value
}

#[test]
fn truncated_callback_never_dereferences_process_pointer() {
    let end = std::mem::offset_of!(CF_CALLBACK_INFO, ProcessInfo)
        + std::mem::size_of::<*const CF_PROCESS_INFO>();
    for size in [0, end - 1, std::mem::size_of::<CF_CALLBACK_INFO>() - 1] {
        let mut value = callback(1_usize as *const CF_PROCESS_INFO);
        value.StructSize = size as u32;
        value.VolumeGuidName = PCWSTR(1_usize as *const u16);
        assert_eq!(
            unsafe { Request::new(value) }.unwrap_err(),
            MetadataError::TruncatedCallback
        );
    }
}

#[test]
fn absent_process_metadata_has_no_identity() {
    let request = unsafe { Request::new(callback(std::ptr::null())) }.unwrap();
    assert!(request.process_identity().is_none());
    assert!(request.process().is_none());
}

#[test]
fn incomplete_process_metadata_has_no_identity() {
    let id_end = std::mem::offset_of!(CF_PROCESS_INFO, ProcessId) + std::mem::size_of::<u32>();
    let session_end = std::mem::offset_of!(CF_PROCESS_INFO, SessionId) + std::mem::size_of::<u32>();
    for size in [
        0,
        std::mem::size_of::<u32>() - 1,
        id_end - 1,
        session_end - 1,
    ] {
        let mut value = process(12345, 6789);
        value.StructSize = size as u32;
        let request = unsafe { Request::new(callback(&value)) }.unwrap();
        assert!(request.process_identity().is_none());
    }
}

#[test]
fn zero_process_id_has_no_identity() {
    let value = process(0, 6789);
    let request = unsafe { Request::new(callback(&value)) }.unwrap();
    assert!(request.process_identity().is_none());
}

#[test]
fn session_zero_remains_valid_scalar_data() {
    let value = process(12345, 0);
    let snapshot = unsafe { Request::new(callback(&value)) }
        .unwrap()
        .process_identity()
        .unwrap();
    assert_eq!(snapshot.id(), 12345);
    assert_eq!(snapshot.session_id(), 0);
}

#[test]
fn snapshot_owns_scalars_and_redacts_debug() {
    let mut source = Box::new(process(12345, 6789));
    let request = unsafe { Request::new(callback(&*source)) }.unwrap();
    source.ProcessId = 54321;
    source.SessionId = 9876;
    drop(source);
    let snapshot = request.process_identity().unwrap();
    assert_eq!(format!("{request:?}"), "Request { .. }");
    assert_eq!(
        format!("{:?}", request.process().unwrap()),
        "Process { .. }"
    );
    drop(request);
    assert_eq!(snapshot.id(), 12345);
    assert_eq!(snapshot.session_id(), 6789);
    assert_eq!(format!("{snapshot:?}"), "ProcessIdentity { .. }");
}

#[test]
fn all_payloads_survive_mutation_free_and_thread_transfer_before_first_accessor() {
    let mut guid = wide("GUID-sensitive");
    let mut dos = wide("Z:");
    let mut normalized = wide("owned-file");
    let mut image = wide("image-sensitive.exe");
    let mut command = wide("command-sensitive --argument");
    let mut package = wide("package-sensitive");
    let mut application = wide("application-sensitive");
    let mut file = vec![1_u8, 2, 3];
    let mut root = vec![4_u8, 5, 6];
    let mut source = Box::new(process(12345, 6789));
    source.ImagePath = PCWSTR(image.as_ptr());
    source.CommandLine = PCWSTR(command.as_ptr());
    source.PackageName = PCWSTR(package.as_ptr());
    source.ApplicationId = PCWSTR(application.as_ptr());
    let mut info = callback(&*source);
    info.VolumeGuidName = PCWSTR(guid.as_ptr());
    info.VolumeDosName = PCWSTR(dos.as_ptr());
    info.NormalizedPath = PCWSTR(normalized.as_ptr());
    info.FileIdentity = file.as_ptr().cast();
    info.FileIdentityLength = file.len() as u32;
    info.SyncRootIdentity = root.as_ptr().cast();
    info.SyncRootIdentityLength = root.len() as u32;
    info.VolumeSerialNumber = 91;
    info.SyncRootFileId = 92;
    info.FileId = 93;
    info.FileSize = 94;
    info.PriorityHint = 7;
    info.ConnectionKey.0 = 95;
    info.TransferKey = 96;
    let request = unsafe { Request::new(info) }.unwrap();

    for buffer in [
        &mut guid,
        &mut dos,
        &mut normalized,
        &mut image,
        &mut command,
        &mut package,
        &mut application,
    ] {
        buffer.fill(88);
    }
    file.fill(99);
    root.fill(99);
    source.ProcessId = 54321;
    source.SessionId = 9876;
    drop((
        guid,
        dos,
        normalized,
        image,
        command,
        package,
        application,
        file,
        root,
        source,
    ));

    std::thread::spawn(move || {
        assert_eq!(request.volume_guid_path(), OsString::from("GUID-sensitive"));
        assert_eq!(request.volume_letter(), OsString::from("Z:"));
        let mut expected = PathBuf::from("Z:");
        expected.push("owned-file");
        assert_eq!(request.path(), expected);
        assert_eq!(request.file_blob(), &[1, 2, 3]);
        assert_eq!(request.register_blob(), &[4, 5, 6]);
        assert_eq!(request.volume_serial_number(), 91);
        assert_eq!(request.sync_root_file_id(), 92);
        assert_eq!(request.file_id(), 93);
        assert_eq!(request.file_size(), 94);
        assert_eq!(request.priority_hint(), 7);
        assert_eq!(request.connection_key(), 95);
        assert_eq!(request.transfer_key(), 96);
        let process = request.process().unwrap();
        assert_eq!(process.id(), 12345);
        assert_eq!(process.session_id(), Some(6789));
        assert_eq!(process.path(), Some(PathBuf::from("image-sensitive.exe")));
        assert_eq!(
            process.command_line(),
            Some(OsString::from("command-sensitive --argument"))
        );
        assert_eq!(process.name(), Some(OsString::from("package-sensitive")));
        assert_eq!(
            process.application_id(),
            Some(OsString::from("application-sensitive"))
        );
        assert_eq!(format!("{process:?}"), "Process { .. }");
    })
    .join()
    .unwrap();
}

#[test]
fn process_older_size_does_not_read_absent_pointer_fields() {
    let mut value = process(12345, 6789);
    value.StructSize =
        (std::mem::offset_of!(CF_PROCESS_INFO, ProcessId) + std::mem::size_of::<u32>()) as u32;
    value.ImagePath = PCWSTR(1_usize as *const u16);
    value.PackageName = PCWSTR(1_usize as *const u16);
    value.ApplicationId = PCWSTR(1_usize as *const u16);
    value.CommandLine = PCWSTR(1_usize as *const u16);
    let request = unsafe { Request::new(callback(&value)) }.unwrap();
    let captured = request.process().unwrap();
    assert_eq!(captured.id(), 12345);
    assert_eq!(captured.session_id(), None);
    assert_eq!(captured.path(), None);
    assert_eq!(captured.name(), None);
    assert_eq!(captured.application_id(), None);
    assert_eq!(captured.command_line(), None);
    assert!(request.process_identity().is_none());
}

#[test]
fn unknown_and_null_optional_process_fields_are_absent() {
    let unknown = wide("UNKNOWN");
    let mut value = process(12345, 0);
    value.ImagePath = PCWSTR(unknown.as_ptr());
    value.CommandLine = PCWSTR(unknown.as_ptr());
    value.PackageName = PCWSTR(unknown.as_ptr());
    value.ApplicationId = PCWSTR(unknown.as_ptr());
    let captured = unsafe { Request::new(callback(&value)) }
        .unwrap()
        .process()
        .unwrap();
    assert!(captured.path().is_none());
    assert!(captured.command_line().is_none());
    assert!(captured.name().is_none());
    assert!(captured.application_id().is_none());

    let value = process(12345, 0);
    let captured = unsafe { Request::new(callback(&value)) }
        .unwrap()
        .process()
        .unwrap();
    assert!(captured.path().is_none());
    assert!(captured.command_line().is_none());
    assert!(captured.name().is_none());
    assert!(captured.application_id().is_none());
}

#[test]
fn mandatory_null_strings_are_rejected() {
    for field in 0..3 {
        let mut value = callback(std::ptr::null());
        match field {
            0 => value.VolumeGuidName = PCWSTR::null(),
            1 => value.VolumeDosName = PCWSTR::null(),
            _ => value.NormalizedPath = PCWSTR::null(),
        }
        assert_eq!(
            unsafe { Request::new(value) }.unwrap_err(),
            MetadataError::InvalidString
        );
    }
}

#[test]
fn negative_file_size_is_rejected() {
    let mut value = callback(std::ptr::null());
    value.FileSize = -1;
    assert_eq!(
        unsafe { Request::new(value) }.unwrap_err(),
        MetadataError::InvalidScalar
    );
}

#[test]
fn null_zero_blobs_are_empty_but_null_nonzero_is_rejected() {
    let request = unsafe { Request::new(callback(std::ptr::null())) }.unwrap();
    assert!(request.file_blob().is_empty());
    assert!(request.register_blob().is_empty());

    for file in [true, false] {
        let mut value = callback(std::ptr::null());
        if file {
            value.FileIdentityLength = 1;
        } else {
            value.SyncRootIdentityLength = 1;
        }
        assert_eq!(
            unsafe { Request::new(value) }.unwrap_err(),
            MetadataError::InvalidBlob
        );
    }
}

#[test]
fn oversize_blobs_refuse_before_reading_sentinel_payloads() {
    for file in [true, false] {
        let mut value = callback(std::ptr::null());
        if file {
            value.FileIdentity = 1_usize as *const core::ffi::c_void;
            value.FileIdentityLength = 4097;
        } else {
            value.SyncRootIdentity = 1_usize as *const core::ffi::c_void;
            value.SyncRootIdentityLength = 65537;
        }
        assert_eq!(
            unsafe { Request::new(value) }.unwrap_err(),
            MetadataError::InvalidBlob
        );
    }
}

#[test]
fn blob_pointer_range_overflow_is_rejected_without_reading() {
    let pointer = (usize::MAX - 1) as *const core::ffi::c_void;
    assert_eq!(
        unsafe { copy_blob(pointer, 4, 4) },
        Err(MetadataError::InvalidBlob)
    );
    assert_eq!(
        unsafe { copy_blob(std::ptr::null(), usize::MAX, usize::MAX) },
        Err(MetadataError::InvalidBlob)
    );
}

#[test]
fn bounded_strings_require_terminator_inside_limit() {
    let mut buffer = vec![65_u16; MAX_WIDE_UNITS];
    assert_eq!(
        unsafe { copy_wide(PCWSTR(buffer.as_ptr()), true) },
        Err(MetadataError::InvalidString)
    );
    buffer[MAX_WIDE_UNITS - 1] = 0;
    assert_eq!(
        unsafe { copy_wide(PCWSTR(buffer.as_ptr()), true) }
            .unwrap()
            .unwrap()
            .len(),
        MAX_WIDE_UNITS - 1
    );
}

#[test]
fn empty_strings_remain_present() {
    let empty = [0_u16];
    assert_eq!(
        unsafe { copy_wide(PCWSTR(empty.as_ptr()), true) },
        Ok(Some(Vec::new()))
    );
    assert_eq!(unsafe { copy_wide(PCWSTR::null(), false) }, Ok(None));
}

#[test]
fn maximum_blob_lengths_are_accepted_and_owned() {
    let file = vec![17_u8; 4096];
    let root = vec![18_u8; 65536];
    let mut value = callback(std::ptr::null());
    value.FileIdentity = file.as_ptr().cast();
    value.FileIdentityLength = file.len() as u32;
    value.SyncRootIdentity = root.as_ptr().cast();
    value.SyncRootIdentityLength = root.len() as u32;
    let request = unsafe { Request::new(value) }.unwrap();
    drop((file, root));
    assert_eq!(request.file_blob(), &[17_u8; 4096]);
    assert_eq!(request.register_blob(), &[18_u8; 65536]);
}

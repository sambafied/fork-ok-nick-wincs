use std::{ffi::OsString, path::PathBuf, slice};

use widestring::{u16cstr, U16CStr};
use windows::Win32::Storage::CloudFilters::{CF_CALLBACK_INFO, CF_PROCESS_INFO};

pub type RawConnectionKey = i64;
pub type RawTransferKey = i64;

/// A struct containing standard information for the current file operation.
pub struct Request(CF_CALLBACK_INFO, Option<ProcessIdentity>);

impl Request {
    pub(crate) fn new(info: CF_CALLBACK_INFO) -> Self {
        let process_identity = Self::capture_process_identity(&info);
        Self(info, process_identity)
    }

    /// The GUID path of the current volume.
    ///
    /// The returned value comes in the form `\?\Volume{GUID}`.
    pub fn volume_guid_path(&self) -> OsString {
        unsafe { U16CStr::from_ptr_str(self.0.VolumeGuidName.0) }.to_os_string()
    }

    /// The letter of the current volume.
    ///
    /// The returned value comes in the form `X:`, where `X` is the drive letter.
    pub fn volume_letter(&self) -> OsString {
        unsafe { U16CStr::from_ptr_str(self.0.VolumeDosName.0) }.to_os_string()
    }

    /// The serial number of the current volume.
    pub fn volume_serial_number(&self) -> u32 {
        self.0.VolumeSerialNumber
    }

    /// Information of the user process that triggered the callback.
    pub fn process(&self) -> Process {
        Process(unsafe { *self.0.ProcessInfo })
    }

    /// Returns the owned scalar process DATA captured at callback entry.
    ///
    /// Missing or undersized metadata and a zero PID return `None`. The result
    /// contains no native strings or pointers and proves neither process birth
    /// nor continuing lifetime, image identity, ownership or cleanup authority.
    /// This accessor never reads the original native process metadata.
    pub fn process_identity(&self) -> Option<ProcessIdentity> {
        self.1
    }

    // Request::new calls this while the original native callback is valid,
    // before dispatch can outlive its process metadata.
    fn capture_process_identity(info: &CF_CALLBACK_INFO) -> Option<ProcessIdentity> {
        let callback_end = std::mem::offset_of!(CF_CALLBACK_INFO, ProcessInfo)
            + std::mem::size_of::<*const CF_PROCESS_INFO>();
        if (info.StructSize as usize) < callback_end {
            return None;
        }
        let process = info.ProcessInfo;
        if process.is_null() {
            return None;
        }
        // Native callback metadata guarantees a readable size header when the
        // pointer is present. Do not form a reference or copy its pointer fields.
        let size = unsafe { std::ptr::addr_of!((*process).StructSize).read_unaligned() };
        let id_end = std::mem::offset_of!(CF_PROCESS_INFO, ProcessId) + std::mem::size_of::<u32>();
        let session_end =
            std::mem::offset_of!(CF_PROCESS_INFO, SessionId) + std::mem::size_of::<u32>();
        if (size as usize) < id_end.max(session_end) {
            return None;
        }
        let id = unsafe { std::ptr::addr_of!((*process).ProcessId).read_unaligned() };
        let session_id = unsafe { std::ptr::addr_of!((*process).SessionId).read_unaligned() };
        (id != 0).then_some(ProcessIdentity { id, session_id })
    }

    /// The NTFS file ID of the sync root folder under which the placeholder being operated on
    /// resides.
    pub fn sync_root_file_id(&self) -> i64 {
        self.0.SyncRootFileId
    }

    /// The NTFS file ID of the placeholder file/directory.
    pub fn file_id(&self) -> i64 {
        self.0.FileId
    }

    /// The logical size of the placeholder file.
    ///
    /// If the placeholder is a directory, this value will always equal 0.
    pub fn file_size(&self) -> u64 {
        self.0.FileSize as u64
    }

    // For now this should be cached on creation
    /// The absolute path of the placeholder file/directory starting from the root directory of the
    /// volume.
    ///
    /// [Read here for more information on this
    /// function.](https://docs.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_callback_info#remarks)
    pub fn path(&self) -> PathBuf {
        let mut path =
            PathBuf::from(unsafe { U16CStr::from_ptr_str(self.0.VolumeDosName.0) }.to_os_string());
        path.push(unsafe { U16CStr::from_ptr_str(self.0.NormalizedPath.0) }.to_os_string());

        path
    }

    /// A numeric scale ranging from
    /// 0-[15](https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/Storage/CloudFilters/constant.CF_MAX_PRIORITY_HINT.html)
    /// to describe the priority of the file operation.
    ///
    /// [Currently, this value does not
    /// change.](https://docs.microsoft.com/en-us/answers/questions/798674/priority-in-cf-callback-info.html)
    pub fn priority_hint(&self) -> u8 {
        self.0.PriorityHint
    }

    // https://docs.microsoft.com/en-us/answers/questions/749979/what-is-a-requestkey-cfapi.html
    // pub fn request_key(&self) -> i64 {
    //     self.0.RequestKey
    // }

    /// The byte slice assigned to the current placeholder file/directory.
    pub fn file_blob(&self) -> &[u8] {
        unsafe {
            slice::from_raw_parts(
                self.0.FileIdentity as *mut u8,
                self.0.FileIdentityLength as usize,
            )
        }
    }

    /// The byte slice assigned to the current sync root on registration.
    pub fn register_blob(&self) -> &[u8] {
        unsafe {
            slice::from_raw_parts(
                self.0.SyncRootIdentity as *mut u8,
                self.0.SyncRootIdentityLength as usize,
            )
        }
    }

    // // https://docs.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_callback_type#remarks
    // // after 60 seconds of no report, windows will cancel the request with an error,
    // // this function is a "hack" to avoid the timeout
    // // https://docs.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfexecute#remarks
    // // CfExecute will reset any timers as stated
    // /// By default, the operating system will invalidate the callback after 60 seconds of no
    // /// activity (meaning, no placeholder methods are invoked). If you are prone to this issue,
    // /// consider calling this method or call placeholder methods more frequently.
    // pub fn reset_timeout() {}

    /// A raw connection key used to identify the connection.
    pub(crate) fn connection_key(&self) -> RawConnectionKey {
        self.0.ConnectionKey.0
    }

    /// A raw transfer key used to identify the current file operation.
    /// Scalar transfer identity for matching cancellation to already-owned work.
    /// This value alone grants no right to construct or execute an operation.
    pub fn transfer_key(&self) -> RawTransferKey {
        self.0.TransferKey
    }
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("volume_guid_path", &self.volume_guid_path())
            .field("volume_letter", &self.volume_letter())
            .field("volume_serial_number", &self.volume_serial_number())
            .field("process", &self.process())
            .field("sync_root_file_id", &self.sync_root_file_id())
            .field("file_id", &self.file_id())
            .field("file_size", &self.file_size())
            .field("path", &self.path())
            .field("priority_hint", &self.priority_hint())
            .finish()
    }
}

/// Owned scalar callback process DATA; not a process handle or authority.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ProcessIdentity {
    id: u32,
    session_id: u32,
}

impl ProcessIdentity {
    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn session_id(&self) -> u32 {
        self.session_id
    }
}

impl std::fmt::Debug for ProcessIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessIdentity").finish_non_exhaustive()
    }
}

/// Information about the calling process.
pub struct Process(CF_PROCESS_INFO);

impl Process {
    /// The application's package name.
    pub fn name(&self) -> OsString {
        unsafe { U16CStr::from_ptr_str(self.0.PackageName.0) }.to_os_string()
    }

    /// The ID of the user process.
    pub fn id(&self) -> u32 {
        self.0.ProcessId
    }

    /// The ID of the session where the user process resides.
    ///
    /// ## Note
    ///
    /// [Process::session_id] is valid in versions 1803 and later.
    pub fn session_id(&self) -> u32 {
        self.0.SessionId
    }

    /// The application's ID.
    pub fn application_id(&self) -> OsString {
        unsafe { U16CStr::from_ptr_str(self.0.ApplicationId.0) }.to_os_string()
    }

    /// The exact command used to initialize the user process.
    ///
    /// ## Note
    ///
    /// [Process::command_line] is valid in versions 1803 and later.
    pub fn command_line(&self) -> Option<OsString> {
        let cmd = unsafe { U16CStr::from_ptr_str(self.0.ImagePath.0) };
        (cmd != u16cstr!("UNKNOWN")).then(|| cmd.to_os_string())
    }

    // TODO: Could be optimized
    /// The absolute path to the main executable file of the process in the format of an NT path.
    ///
    /// This function returns [None][std::option::Option::None] when the operating system failed to
    /// retrieve the path.
    pub fn path(&self) -> Option<PathBuf> {
        let path = unsafe { U16CStr::from_ptr_str(self.0.ImagePath.0) };
        (path != u16cstr!("UNKNOWN")).then(|| PathBuf::from(path.to_os_string()))
    }
}

impl std::fmt::Debug for Process {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Process")
            .field("name", &self.name())
            .field("id", &self.id())
            .field("session_id", &self.session_id())
            .field("application_id", &self.application_id())
            .field("command_line", &self.command_line())
            .field("path", &self.path())
            .finish()
    }
}

#[cfg(test)]
mod process_identity_tests {
    use super::*;

    fn process(id: u32, session_id: u32) -> CF_PROCESS_INFO {
        // These Windows ABI structs contain scalar values and nullable pointers.
        let mut value: CF_PROCESS_INFO = unsafe { std::mem::zeroed() };
        value.StructSize = std::mem::size_of::<CF_PROCESS_INFO>() as u32;
        value.ProcessId = id;
        value.SessionId = session_id;
        value
    }

    fn callback(process: *const CF_PROCESS_INFO) -> CF_CALLBACK_INFO {
        let mut value: CF_CALLBACK_INFO = unsafe { std::mem::zeroed() };
        value.StructSize = std::mem::size_of::<CF_CALLBACK_INFO>() as u32;
        value.ProcessInfo = process as _;
        value
    }

    #[test]
    fn truncated_callback_never_dereferences_process_pointer() {
        // Deliberately unreadable: a truncated callback must reject before use.
        let mut value = callback(1_usize as *const CF_PROCESS_INFO);
        let end = std::mem::offset_of!(CF_CALLBACK_INFO, ProcessInfo)
            + std::mem::size_of::<*const CF_PROCESS_INFO>();
        for size in [0, end - 1] {
            value.StructSize = size as u32;
            assert!(Request::new(value).process_identity().is_none());
        }
    }

    #[test]
    fn absent_process_metadata_has_no_identity() {
        assert!(Request::new(callback(std::ptr::null()))
            .process_identity()
            .is_none());
    }

    #[test]
    fn incomplete_process_metadata_has_no_identity() {
        let mut value = process(12345, 6789);
        let id_end = std::mem::offset_of!(CF_PROCESS_INFO, ProcessId) + std::mem::size_of::<u32>();
        let session_end =
            std::mem::offset_of!(CF_PROCESS_INFO, SessionId) + std::mem::size_of::<u32>();
        for size in [
            0,
            std::mem::size_of::<u32>() - 1,
            id_end - 1,
            session_end - 1,
        ] {
            value.StructSize = size as u32;
            assert!(Request::new(callback(&value)).process_identity().is_none());
        }
    }

    #[test]
    fn zero_process_id_has_no_identity() {
        let value = process(0, 6789);
        assert!(Request::new(callback(&value)).process_identity().is_none());
    }

    #[test]
    fn session_zero_remains_valid_scalar_data() {
        let value = process(12345, 0);
        let snapshot = Request::new(callback(&value)).process_identity().unwrap();
        assert_eq!(snapshot.id(), 12345);
        assert_eq!(snapshot.session_id(), 0);
    }

    #[test]
    fn snapshot_owns_scalars_and_redacts_debug() {
        let mut source = Box::new(process(12345, 6789));
        let request = Request::new(callback(&*source));
        source.ProcessId = 54321;
        source.SessionId = 9876;
        drop(source);
        let snapshot = request.process_identity().unwrap();
        drop(request);
        assert_eq!(snapshot.id(), 12345);
        assert_eq!(snapshot.session_id(), 6789);
        assert_eq!(format!("{snapshot:?}"), "ProcessIdentity { .. }");
    }
}

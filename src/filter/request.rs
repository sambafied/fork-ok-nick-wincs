use std::{ffi::OsString, path::PathBuf, slice};

use widestring::{u16cstr, U16CStr};
use windows::Win32::Storage::CloudFilters::{CF_CALLBACK_INFO, CF_PROCESS_INFO};

pub type RawConnectionKey = i64;
pub type RawTransferKey = i64;

/// A struct containing standard information for the current file operation.
pub struct Request(CF_CALLBACK_INFO, super::OperationContext);

impl Request {
    pub(crate) fn new(mut info: CF_CALLBACK_INFO) -> Self {
        let context = super::OperationContext::from_callback(&info);
        info.CorrelationVector = std::ptr::null();
        Self(info, context)
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

    /// Clone the operation values already owned when this request was constructed.
    pub fn operation_context(&self) -> super::OperationContext {
        self.1.clone()
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
mod operation_context_tests {
    use super::*;
    use windows::Win32::{
        Storage::CloudFilters::{CF_CONNECTION_KEY, CF_OPERATION_TYPE_TRANSFER_DATA},
        System::CorrelationVector::CORRELATION_VECTOR,
    };

    #[test]
    fn operation_context_survives_the_original_callback_correlation_storage() {
        let request = {
            let mut correlation = CORRELATION_VECTOR::default();
            correlation.Vector[0] = 42;
            let mut info = CF_CALLBACK_INFO {
                ConnectionKey: CF_CONNECTION_KEY(7),
                TransferKey: 11,
                RequestKey: 17,
                CorrelationVector: &correlation,
                ..Default::default()
            };
            let request = Request::new(info);
            correlation.Vector[0] = 9;
            info.CorrelationVector = std::ptr::null();
            assert!(info.CorrelationVector.is_null());
            assert_eq!(correlation.Vector[0], 9);
            request
        };
        assert!(request.0.CorrelationVector.is_null());
        let context = request.operation_context();
        let operation = context.operation_info(CF_OPERATION_TYPE_TRANSFER_DATA);
        assert_eq!(operation.RequestKey, 17);
        assert_eq!(unsafe { (*operation.CorrelationVector).Vector[0] }, 42);
    }
}

use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};

use windows::Win32::Storage::CloudFilters::{CF_CALLBACK_INFO, CF_PROCESS_INFO};

use super::metadata::{copy_blob, copy_wide, MetadataError};

pub type RawConnectionKey = i64;
pub type RawTransferKey = i64;

/// Owned metadata for a file operation, captured before asynchronous dispatch.
pub struct Request {
    volume_guid: Vec<u16>,
    volume_dos: Vec<u16>,
    normalized_path: Vec<u16>,
    volume_serial: u32,
    sync_root_file_id: i64,
    file_id: i64,
    file_size: u64,
    priority_hint: u8,
    file_blob: Vec<u8>,
    register_blob: Vec<u8>,
    connection_key: RawConnectionKey,
    transfer_key: RawTransferKey,
    process: Option<Process>,
    process_identity: Option<ProcessIdentity>,
}

impl Request {
    /// Capture while the native callback and its buffers remain readable.
    ///
    /// # Safety
    ///
    /// The caller must validate the native callback pointer and its size before
    /// copying the ABI value passed here. Non-null payload pointers must refer
    /// to readable SDK buffers for their declared extent. Bounded scans cannot
    /// validate arbitrary forged addresses.
    pub(crate) unsafe fn new(info: CF_CALLBACK_INFO) -> Result<Self, MetadataError> {
        if (info.StructSize as usize) < std::mem::size_of::<CF_CALLBACK_INFO>() {
            return Err(MetadataError::TruncatedCallback);
        }
        let file_size = u64::try_from(info.FileSize).map_err(|_| MetadataError::InvalidScalar)?;
        let volume_guid =
            unsafe { copy_wide(info.VolumeGuidName, true)? }.ok_or(MetadataError::InvalidString)?;
        let volume_dos =
            unsafe { copy_wide(info.VolumeDosName, true)? }.ok_or(MetadataError::InvalidString)?;
        let normalized_path =
            unsafe { copy_wide(info.NormalizedPath, true)? }.ok_or(MetadataError::InvalidString)?;
        let file_blob =
            unsafe { copy_blob(info.FileIdentity, info.FileIdentityLength as usize, 4096)? };
        let register_blob = unsafe {
            copy_blob(
                info.SyncRootIdentity,
                info.SyncRootIdentityLength as usize,
                65536,
            )?
        };
        let process = unsafe { Process::capture(info.ProcessInfo)? };
        let process_identity = process.as_ref().and_then(|process| {
            process.session_id.and_then(|session_id| {
                (process.id != 0).then_some(ProcessIdentity {
                    id: process.id,
                    session_id,
                })
            })
        });
        Ok(Self {
            volume_guid,
            volume_dos,
            normalized_path,
            volume_serial: info.VolumeSerialNumber,
            sync_root_file_id: info.SyncRootFileId,
            file_id: info.FileId,
            file_size,
            priority_hint: info.PriorityHint,
            file_blob,
            register_blob,
            connection_key: info.ConnectionKey.0,
            transfer_key: info.TransferKey,
            process,
            process_identity,
        })
    }

    /// The GUID path of the current volume.
    pub fn volume_guid_path(&self) -> OsString {
        OsString::from_wide(&self.volume_guid)
    }

    /// The DOS name of the current volume.
    pub fn volume_letter(&self) -> OsString {
        OsString::from_wide(&self.volume_dos)
    }

    /// Borrow captured UTF-16 for fallible callback-info snapshots.
    pub(crate) fn volume_utf16(&self) -> &[u16] {
        &self.volume_dos
    }

    pub fn volume_serial_number(&self) -> u32 {
        self.volume_serial
    }

    /// Owned nullable process metadata; conveys no process authority.
    pub fn process(&self) -> Option<Process> {
        self.process.clone()
    }

    /// Owned scalar DATA, not proof of process lifetime, identity or ownership.
    pub fn process_identity(&self) -> Option<ProcessIdentity> {
        self.process_identity
    }

    pub fn sync_root_file_id(&self) -> i64 {
        self.sync_root_file_id
    }

    pub fn file_id(&self) -> i64 {
        self.file_id
    }

    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    pub fn path(&self) -> PathBuf {
        let mut path = PathBuf::from(OsString::from_wide(&self.volume_dos));
        path.push(OsString::from_wide(&self.normalized_path));
        path
    }

    pub fn priority_hint(&self) -> u8 {
        self.priority_hint
    }

    pub fn file_blob(&self) -> &[u8] {
        &self.file_blob
    }

    pub fn register_blob(&self) -> &[u8] {
        &self.register_blob
    }

    pub(crate) fn connection_key(&self) -> RawConnectionKey {
        self.connection_key
    }

    pub(crate) fn transfer_key(&self) -> RawTransferKey {
        self.transfer_key
    }
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request").finish_non_exhaustive()
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

/// Owned nullable metadata about the calling process.
#[derive(Clone)]
pub struct Process {
    id: u32,
    session_id: Option<u32>,
    image_path: Option<Vec<u16>>,
    package_name: Option<Vec<u16>>,
    application_id: Option<Vec<u16>>,
    command_line: Option<Vec<u16>>,
}

impl Process {
    // Do not copy the native struct: older versions may lack its tail.
    unsafe fn capture(pointer: *const CF_PROCESS_INFO) -> Result<Option<Self>, MetadataError> {
        if pointer.is_null() {
            return Ok(None);
        }
        let size = unsafe { std::ptr::addr_of!((*pointer).StructSize).read_unaligned() } as usize;
        let id_end = std::mem::offset_of!(CF_PROCESS_INFO, ProcessId) + std::mem::size_of::<u32>();
        if size < id_end {
            return Ok(None);
        }
        let id = unsafe { std::ptr::addr_of!((*pointer).ProcessId).read_unaligned() };

        // Only evaluate a field address/read when StructSize covers that field.
        macro_rules! string_field {
            ($field:ident) => {{
                let end = std::mem::offset_of!(CF_PROCESS_INFO, $field)
                    + std::mem::size_of::<windows::core::PCWSTR>();
                if size >= end {
                    let value = unsafe { std::ptr::addr_of!((*pointer).$field).read_unaligned() };
                    let copied = unsafe { copy_wide(value, false)? };
                    copied.filter(|value| !is_unknown(value))
                } else {
                    None
                }
            }};
        }

        let image_path = string_field!(ImagePath);
        let package_name = string_field!(PackageName);
        let application_id = string_field!(ApplicationId);
        let command_line = string_field!(CommandLine);
        let session_end =
            std::mem::offset_of!(CF_PROCESS_INFO, SessionId) + std::mem::size_of::<u32>();
        let session_id = if size >= session_end {
            Some(unsafe { std::ptr::addr_of!((*pointer).SessionId).read_unaligned() })
        } else {
            None
        };
        Ok(Some(Self {
            id,
            session_id,
            image_path,
            package_name,
            application_id,
            command_line,
        }))
    }

    pub fn name(&self) -> Option<OsString> {
        self.package_name.as_deref().map(OsString::from_wide)
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    /// Absent when the native version does not cover SessionId.
    pub fn session_id(&self) -> Option<u32> {
        self.session_id
    }

    pub fn application_id(&self) -> Option<OsString> {
        self.application_id.as_deref().map(OsString::from_wide)
    }

    /// Uses CommandLine, independently of ImagePath.
    pub fn command_line(&self) -> Option<OsString> {
        self.command_line.as_deref().map(OsString::from_wide)
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.image_path
            .as_deref()
            .map(OsString::from_wide)
            .map(PathBuf::from)
    }
}

fn is_unknown(value: &[u16]) -> bool {
    value == [85, 78, 75, 78, 79, 87, 78]
}

impl std::fmt::Debug for Process {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Process").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "request_owned_tests.rs"]
mod process_identity_tests;

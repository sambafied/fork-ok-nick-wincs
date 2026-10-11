use std::{
    fmt::Debug,
    fs::File,
    mem::{self, ManuallyDrop},
    ops::{Bound, Range, RangeBounds},
    os::windows::io::{AsRawHandle, FromRawHandle, IntoRawHandle, RawHandle},
    path::Path,
    ptr,
    sync::Arc,
};

use widestring::U16CString;
use windows::{
    core::{self, PCWSTR},
    Win32::{
        Foundation::{
            CloseHandle, ERROR_CLOUD_FILE_NOT_IN_SYNC, ERROR_NOT_A_CLOUD_FILE, E_HANDLE,
            E_INVALIDARG, E_UNEXPECTED, HANDLE,
        },
        Storage::CloudFilters::{
            self, CfCloseHandle, CfConvertToPlaceholder, CfGetPlaceholderInfo,
            CfGetPlaceholderRangeInfo, CfGetWin32HandleFromProtectedHandle, CfHydratePlaceholder,
            CfOpenFileWithOplock, CfReferenceProtectedHandle, CfReleaseProtectedHandle,
            CfSetInSyncState, CfSetPinState, CfUpdatePlaceholder, CF_CONVERT_FLAGS, CF_FILE_RANGE,
            CF_OPEN_FILE_FLAGS, CF_PIN_STATE, CF_PLACEHOLDER_RANGE_INFO_CLASS,
            CF_PLACEHOLDER_STANDARD_INFO, CF_SET_PIN_FLAGS, CF_UPDATE_FLAGS,
        },
        Storage::FileSystem::{FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO},
        System::{
            Ioctl::{FSCTL_READ_FILE_USN_DATA, READ_FILE_USN_DATA, USN_RECORD_V2},
            IO::DeviceIoControl,
        },
    },
};

use crate::{metadata::Metadata, usn::Usn};

/// The type of handle that the placeholder file/directory owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceholderHandleType {
    /// A handle that was opened with [CfOpenFileWithOplock].
    CfApi,
    /// A handle that was opened with [CreateFileW][windows::Win32::Storage::FileSystem::CreateFileW] etc.
    Win32,
}

/// An owned handle to a placeholder file/directory.
///
/// This closes the handle on drop.
#[derive(Debug)]
pub struct OwnedPlaceholderHandle {
    handle_type: PlaceholderHandleType,
    handle: HANDLE,
}

impl OwnedPlaceholderHandle {
    /// Create a new [OwnedPlaceholderHandle] from a handle returned by [CfOpenFileWithOplock].
    ///
    /// # Safety
    ///
    /// The handle must be valid and owned by the caller.
    pub unsafe fn from_cfapi(handle: HANDLE) -> Self {
        Self {
            handle_type: PlaceholderHandleType::CfApi,
            handle,
        }
    }

    /// Create a new [OwnedPlaceholderHandle] from a handle returned by
    /// [CreateFile][windows::Win32::Storage::FileSystem::CreateFileW] etc.
    ///
    /// # Safety
    ///
    /// The handle must be valid and owned by the caller.
    pub unsafe fn from_win32(handle: HANDLE) -> Self {
        Self {
            handle_type: PlaceholderHandleType::Win32,
            handle,
        }
    }

    pub const fn handle(&self) -> HANDLE {
        self.handle
    }

    pub const fn handle_type(&self) -> PlaceholderHandleType {
        self.handle_type
    }
}

impl Drop for OwnedPlaceholderHandle {
    fn drop(&mut self) {
        match self.handle_type {
            PlaceholderHandleType::CfApi => unsafe { CfCloseHandle(self.handle) },
            PlaceholderHandleType::Win32 => unsafe {
                _ = CloseHandle(self.handle);
            },
        }
    }
}

/// Holds a Win32 handle from the protected handle.
///
/// Clones share one successful native reference. The final clone releases it,
/// then permits the opaque owner to close. Keep these guards short-lived: a
/// reference delays acknowledgement of an oplock break.
#[derive(Clone)]
pub struct ArcWin32Handle {
    inner: Arc<ProtectedReference>,
}

struct ProtectedReference {
    win32_handle: HANDLE,
    owner: Arc<OwnedPlaceholderHandle>,
}

impl ArcWin32Handle {
    /// Win32 handle from the protected handle.
    pub fn handle(&self) -> HANDLE {
        self.inner.win32_handle
    }
}

impl AsRawHandle for ArcWin32Handle {
    fn as_raw_handle(&self) -> RawHandle {
        self.handle().0
    }
}

impl Drop for ProtectedReference {
    fn drop(&mut self) {
        unsafe { CfReleaseProtectedHandle(self.owner.handle) };
    }
}

/// Safety: reference counted by syscall
unsafe impl Send for ProtectedReference {}
/// Safety: reference counted by syscall
unsafe impl Sync for ProtectedReference {}

// Ownership only; methods using a protected Win32 handle acquire a reference.
unsafe impl Send for OwnedPlaceholderHandle {}
unsafe impl Sync for OwnedPlaceholderHandle {}

/// Options for opening a placeholder file/directory.
pub struct OpenOptions {
    flags: CF_OPEN_FILE_FLAGS,
}

impl OpenOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn exclusive(mut self) -> Self {
        self.flags |= CloudFilters::CF_OPEN_FILE_FLAG_EXCLUSIVE;
        self
    }

    pub fn write_access(mut self) -> Self {
        self.flags |= CloudFilters::CF_OPEN_FILE_FLAG_WRITE_ACCESS;
        self
    }

    pub fn delete_access(mut self) -> Self {
        self.flags |= CloudFilters::CF_OPEN_FILE_FLAG_DELETE_ACCESS;
        self
    }

    pub fn foreground(mut self) -> Self {
        self.flags |= CloudFilters::CF_OPEN_FILE_FLAG_FOREGROUND;
        self
    }

    /// Open the placeholder file/directory using `CfOpenFileWithOplock`.
    pub fn open(self, path: impl AsRef<Path>) -> core::Result<Placeholder> {
        let u16_path =
            U16CString::from_os_str(path.as_ref()).map_err(|_| core::Error::from(E_INVALIDARG))?;
        let handle = unsafe { CfOpenFileWithOplock(PCWSTR(u16_path.as_ptr()), self.flags) }?;
        Ok(Placeholder {
            handle: Arc::new(unsafe { OwnedPlaceholderHandle::from_cfapi(handle) }),
        })
    }
}

impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            flags: CloudFilters::CF_OPEN_FILE_FLAG_NONE,
        }
    }
}

/// The pin state of a placeholder.
///
/// [Read more
/// here](https://docs.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_pin_state#remarks)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinState {
    /// The platform could decide freely.
    Unspecified,
    /// [SyncFilter::fetch_data][crate::filter::SyncFilter::fetch_data] will be called to hydrate the rest
    /// of the placeholder's data. Any dehydration requests will fail automatically.
    Pinned,
    /// [SyncFilter::dehydrate][crate::filter::SyncFilter::dehydrate] will be called to dehydrate the rest
    /// of the placeholder's data.
    Unpinned,
    /// The placeholder will never sync to the cloud.
    Excluded,
    /// The placeholder will inherit the parent placeholder's pin state.
    Inherit,
}

impl From<PinState> for CF_PIN_STATE {
    fn from(state: PinState) -> Self {
        match state {
            PinState::Unspecified => CloudFilters::CF_PIN_STATE_UNSPECIFIED,
            PinState::Pinned => CloudFilters::CF_PIN_STATE_PINNED,
            PinState::Unpinned => CloudFilters::CF_PIN_STATE_UNPINNED,
            PinState::Excluded => CloudFilters::CF_PIN_STATE_EXCLUDED,
            PinState::Inherit => CloudFilters::CF_PIN_STATE_INHERIT,
        }
    }
}

impl From<CF_PIN_STATE> for PinState {
    fn from(state: CF_PIN_STATE) -> Self {
        match state {
            CloudFilters::CF_PIN_STATE_UNSPECIFIED => PinState::Unspecified,
            CloudFilters::CF_PIN_STATE_PINNED => PinState::Pinned,
            CloudFilters::CF_PIN_STATE_UNPINNED => PinState::Unpinned,
            CloudFilters::CF_PIN_STATE_EXCLUDED => PinState::Excluded,
            CloudFilters::CF_PIN_STATE_INHERIT => PinState::Inherit,
            _ => unreachable!(),
        }
    }
}

/// The placeholder pin flags.
#[derive(Debug, Clone, Copy)]
pub struct PinOptions(CF_SET_PIN_FLAGS);

impl PinOptions {
    /// Applies the pin state to all descendants of the placeholder (if the placeholder is a
    /// directory).
    pub fn recurse(&mut self) -> &mut Self {
        self.0 |= CloudFilters::CF_SET_PIN_FLAG_RECURSE;
        self
    }

    /// Applies the pin state to all descendants of the placeholder excluding the current one (if
    /// the placeholder is a directory).
    pub fn recurse_children(&mut self) -> &mut Self {
        self.0 |= CloudFilters::CF_SET_PIN_FLAG_RECURSE_ONLY;
        self
    }

    /// Stop applying the pin state when the first error is encountered. Otherwise, skip over it
    /// and keep applying.
    pub fn stop_on_error(&mut self) -> &mut Self {
        self.0 |= CloudFilters::CF_SET_PIN_FLAG_RECURSE_STOP_ON_ERROR;
        self
    }
}

impl Default for PinOptions {
    fn default() -> Self {
        Self(CloudFilters::CF_SET_PIN_FLAG_NONE)
    }
}

/// File to placeholder file conversion parameters.
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    flags: CF_CONVERT_FLAGS,
    blob: Vec<u8>,
}

impl ConvertOptions {
    /// Marks a placeholder as in sync.
    ///
    /// See also
    /// [SetInSyncState](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfsetinsyncstate),
    /// [What does "In-Sync" Mean?](https://www.userfilesystem.com/programming/faq/#nav_whatdoesin-syncmean)
    pub fn mark_in_sync(mut self) -> Self {
        self.flags |= CloudFilters::CF_CONVERT_FLAG_MARK_IN_SYNC;
        self
    }

    /// Dehydrate the placeholder after conversion.
    ///
    /// This flag is only applicable to files.
    pub fn dehydrate(mut self) -> Self {
        self.flags |= CloudFilters::CF_CONVERT_FLAG_DEHYDRATE;
        self
    }

    /// Marks the placeholder as "partially full," such that
    /// [SyncFilter::fetch_placeholders][crate::filter::SyncFilter::fetch_placeholders]
    /// will be invoked when this directory is next accessed so that the remaining placeholders are inserted.
    ///
    /// Only applicable to placeholder directories.
    pub fn has_children(mut self) -> Self {
        self.flags |= CloudFilters::CF_CONVERT_FLAG_ENABLE_ON_DEMAND_POPULATION;
        self
    }

    /// Blocks this placeholder from being dehydrated.
    ///
    /// This flag does not work on directories.
    pub fn block_dehydration(mut self) -> Self {
        self.flags |= CloudFilters::CF_CONVERT_FLAG_ALWAYS_FULL;
        self
    }

    /// Forces the conversion of a non-cloud placeholder file to a cloud placeholder file.
    ///
    /// Placeholder files are built into the NTFS file system and thus, a placeholder not associated
    /// with the sync root is possible.
    pub fn force(mut self) -> Self {
        self.flags |= CloudFilters::CF_CONVERT_FLAG_FORCE_CONVERT_TO_CLOUD_FILE;
        self
    }

    /// A buffer of bytes stored with the file that could be accessed through a
    /// [Request::file_blob][crate::filter::Request::file_blob] or [Placeholder::info].
    ///
    /// The buffer must not exceed
    /// [4KiB](https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/Storage/CloudFilters/constant.CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH.html).
    pub fn blob(mut self, blob: Vec<u8>) -> Self {
        assert!(
            blob.len() <= CloudFilters::CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH as usize,
            "blob size must not exceed {} bytes, got {} bytes",
            CloudFilters::CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH,
            blob.len()
        );
        self.blob = blob;
        self
    }
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            flags: CloudFilters::CF_CONVERT_FLAG_NONE,
            blob: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct PlaceholderInfo {
    info: CF_PLACEHOLDER_STANDARD_INFO,
    blob: Vec<u8>,
}

impl PlaceholderInfo {
    pub fn on_disk_data_size(&self) -> i64 {
        self.info.OnDiskDataSize
    }

    pub fn validated_data_size(&self) -> i64 {
        self.info.ValidatedDataSize
    }

    pub fn modified_data_size(&self) -> i64 {
        self.info.ModifiedDataSize
    }

    pub fn properties_size(&self) -> i64 {
        self.info.PropertiesSize
    }

    pub fn pin_state(&self) -> PinState {
        self.info.PinState.into()
    }

    pub fn is_in_sync(&self) -> bool {
        self.info.InSyncState == CloudFilters::CF_IN_SYNC_STATE_IN_SYNC
    }

    pub fn file_id(&self) -> i64 {
        self.info.FileId
    }

    pub fn sync_root_file_id(&self) -> i64 {
        self.info.SyncRootFileId
    }

    pub fn blob(&self) -> &[u8] {
        &self.blob
    }
}

impl std::fmt::Debug for PlaceholderInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaceholderInfo")
            .field("on_disk_data_size", &self.on_disk_data_size())
            .field("validated_data_size", &self.validated_data_size())
            .field("modified_data_size", &self.modified_data_size())
            .field("properties_size", &self.properties_size())
            .field("pin_state", &self.pin_state())
            .field("is_in_sync", &self.is_in_sync())
            .field("file_id", &self.file_id())
            .field("sync_root_file_id", &self.sync_root_file_id())
            .finish()
    }
}

/// Placeholder update parameters.
#[derive(Debug, Clone)]
pub struct UpdateOptions<'a> {
    metadata: Option<Metadata>,
    dehydrate_ranges: Vec<CF_FILE_RANGE>,
    flags: CF_UPDATE_FLAGS,
    blob: &'a [u8],
}

impl<'a> UpdateOptions<'a> {
    /// [Metadata] contains file system metadata about the placeholder to be updated.
    ///
    /// File size will be truncates to 0 if not specified, otherwise to the specified size.
    pub fn metadata(mut self, metadata: Metadata) -> Self {
        self.flags &= !(CloudFilters::CF_UPDATE_FLAG_PASSTHROUGH_FS_METADATA);
        self.metadata = Some(metadata);
        self
    }

    /// Fields in [Metadata] will be updated.
    pub fn metadata_all(mut self, metadata: Metadata) -> Self {
        self.flags |= CloudFilters::CF_UPDATE_FLAG_PASSTHROUGH_FS_METADATA;
        self.metadata = Some(metadata);
        self
    }

    /// Extended ranges to be dehydrated.
    ///
    /// All the offsets and lengths should be `PAGE_SIZE` aligned.
    /// Passing a single range with Offset `0` and Length `CF_EOF` will invalidate the entire file.
    /// This has the same effect as passing the flag `CF_UPDATE_FLAG_DEHYDRATE` instead
    pub fn dehydrate_ranges(mut self, ranges: impl IntoIterator<Item = Range<u64>>) -> Self {
        self.dehydrate_ranges
            .extend(ranges.into_iter().map(|r| CF_FILE_RANGE {
                StartingOffset: r.start as _,
                Length: (r.end - r.start) as _,
            }));
        self
    }

    /// The update will fail if the `IN_SYNC` attribute is not currently set on the placeholder.
    pub fn update_if_in_sync(mut self) -> Self {
        self.flags |= CloudFilters::CF_UPDATE_FLAG_VERIFY_IN_SYNC;
        self
    }

    /// Marks a placeholder as in sync.
    ///
    /// See also
    /// [SetInSyncState](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfsetinsyncstate),
    /// [What does "In-Sync" Mean?](https://www.userfilesystem.com/programming/faq/#nav_whatdoesin-syncmean)
    pub fn mark_in_sync(mut self) -> Self {
        self.flags |= CloudFilters::CF_UPDATE_FLAG_MARK_IN_SYNC;
        self
    }

    /// Marks a placeholder as not in sync. `Sync Pending` will be shown in explorer.
    ///
    /// See also
    /// [SetInSyncState](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfsetinsyncstate),
    /// [What does "In-Sync" Mean?](https://www.userfilesystem.com/programming/faq/#nav_whatdoesin-syncmean)
    pub fn mark_not_in_sync(mut self) -> Self {
        self.flags |= CloudFilters::CF_UPDATE_FLAG_CLEAR_IN_SYNC;
        self
    }

    /// The platform dehydrates the file after updating the placeholder successfully.
    pub fn dehydrate(mut self) -> Self {
        self.flags |= CloudFilters::CF_UPDATE_FLAG_DEHYDRATE;
        self
    }

    /// Disables on-demand population for directories.
    pub fn has_no_children(mut self) -> Self {
        self.flags |= CloudFilters::CF_UPDATE_FLAG_DISABLE_ON_DEMAND_POPULATION;
        self
    }

    /// Enable on-demand population for directories.
    pub fn has_children(mut self) -> Self {
        self.flags |= CloudFilters::CF_UPDATE_FLAG_ENABLE_ON_DEMAND_POPULATION;
        self
    }

    /// Remove the file identity from the placeholder.
    /// [UpdateOptions::blob()](crate::placeholder::UpdateOptions::blob) will be ignored.
    pub fn remove_blob(mut self) -> Self {
        self.flags |= CloudFilters::CF_UPDATE_FLAG_REMOVE_FILE_IDENTITY;
        self
    }

    /// The platform removes all existing extrinsic properties on the placeholder.
    pub fn remove_properties(mut self) -> Self {
        self.flags |= CloudFilters::CF_UPDATE_FLAG_REMOVE_PROPERTY;
        self
    }

    pub fn blob(mut self, blob: &'a [u8]) -> Self {
        assert!(
            blob.len() <= CloudFilters::CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH as usize,
            "blob size must not exceed {} bytes, got {} bytes",
            CloudFilters::CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH,
            blob.len()
        );
        self.blob = blob;
        self
    }
}

impl Default for UpdateOptions<'_> {
    fn default() -> Self {
        Self {
            metadata: None,
            dehydrate_ranges: Vec::new(),
            flags: CloudFilters::CF_UPDATE_FLAG_NONE,
            blob: &[],
        }
    }
}

/// The type of data to read from a placeholder.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ReadType {
    /// Any data that is saved to the disk.
    Any,
    /// Data that has been synced to the cloud.
    Validated,
    /// Data that has not synced to the cloud.
    Modified,
}

impl From<ReadType> for CF_PLACEHOLDER_RANGE_INFO_CLASS {
    fn from(read_type: ReadType) -> Self {
        match read_type {
            ReadType::Any => CloudFilters::CF_PLACEHOLDER_RANGE_INFO_ONDISK,
            ReadType::Validated => CloudFilters::CF_PLACEHOLDER_RANGE_INFO_VALIDATED,
            ReadType::Modified => CloudFilters::CF_PLACEHOLDER_RANGE_INFO_MODIFIED,
        }
    }
}

// #[derive(Clone, Copy)]
// pub struct PlaceholderState(CF_PLACEHOLDER_STATE);

// impl PlaceholderState {
//     /// The placeholder is both a directory as well as the sync root.
//     pub fn sync_root(&self) -> bool {
//         (self.0 & CloudFilters::CF_PLACEHOLDER_STATE_SYNC_ROOT).0 != 0
//     }

//     /// There exists an essential property in the property store of the file or directory.
//     pub fn essential_prop_present(&self) -> bool {
//         (self.0 & CloudFilters::CF_PLACEHOLDER_STATE_ESSENTIAL_PROP_PRESENT).0 != 0
//     }

//     /// The placeholder is in sync.
//     pub fn in_sync(&self) -> bool {
//         (self.0 & CloudFilters::CF_PLACEHOLDER_STATE_IN_SYNC).0 != 0
//     }

//     /// The placeholder content is not ready to be consumed by the user application,
//     /// though it may or may not be fully present locally.
//     ///
//     /// An example is a placeholder file whose content has been fully downloaded to the local disk,
//     /// but is yet to be validated by a sync provider that
//     /// has registered the sync root with the hydration modifier
//     /// [HydrationPolicy::require_validation][crate::root::HydrationPolicy::require_validation].
//     pub fn partial(&self) -> bool {
//         (self.0 & CloudFilters::CF_PLACEHOLDER_STATE_PARTIAL).0 != 0
//     }

//     /// The placeholder content is not fully present locally.
//     ///
//     /// When this is set, [PlaceholderState::partial] also be `true`.
//     pub fn partial_on_disk(&self) -> bool {
//         (self.0 & CloudFilters::CF_PLACEHOLDER_STATE_PARTIALLY_ON_DISK).0 != 0
//     }
// }

// impl Debug for PlaceholderState {
//     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         f.debug_struct("PlaceholderState")
//             .field("sync_root", &self.sync_root())
//             .field("essential_prop_present", &self.essential_prop_present())
//             .field("in_sync", &self.in_sync())
//             .field("partial", &self.partial())
//             .field("partial_on_disk", &self.partial_on_disk())
//             .finish()
//     }
// }

/// A struct to perform various operations on a placeholder(or regular) file/directory.
#[derive(Debug)]
pub struct Placeholder {
    handle: Arc<OwnedPlaceholderHandle>,
}

/// Identity and nonzero NTFS USN of a local generation. Persist alongside the
/// immutable uploaded snapshot and its hash; this token does not verify a remote
/// upload or provide a snapshot barrier by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileGeneration {
    volume: u64,
    file_id: [u8; 16],
    usn: Usn,
}

impl FileGeneration {
    /// Restore the exact token persisted with an immutable snapshot. This does
    /// not assert that an upload was authorized or remotely verified.
    pub fn from_parts(volume: u64, file_id: [u8; 16], usn: Usn) -> core::Result<Self> {
        if usn <= 0 {
            return Err(E_INVALIDARG.into());
        }
        Ok(Self {
            volume,
            file_id,
            usn,
        })
    }

    pub fn volume(&self) -> u64 {
        self.volume
    }
    pub fn file_id(&self) -> [u8; 16] {
        self.file_id
    }
    pub fn usn(&self) -> Usn {
        self.usn
    }
}

/// A short exclusive acknowledgement barrier. Never retain across network I/O.
/// Dropping without `commit` changes no sync state and releases pending openers.
pub struct UploadAcknowledgement {
    reference: ArcWin32Handle,
    expected: FileGeneration,
}

impl UploadAcknowledgement {
    pub fn begin(path: impl AsRef<Path>, expected: FileGeneration) -> core::Result<Self> {
        let placeholder = Placeholder::options()
            .exclusive()
            .write_access()
            .open(path)?;
        let reference = placeholder.win32_handle()?;
        if generation_for_handle(reference.handle())? != expected {
            return Err(ERROR_CLOUD_FILE_NOT_IN_SYNC.to_hresult().into());
        }
        Ok(Self {
            reference,
            expected,
        })
    }

    pub fn commit(self) -> core::Result<FileGeneration> {
        if generation_for_handle(self.reference.handle())? != self.expected {
            return Err(ERROR_CLOUD_FILE_NOT_IN_SYNC.to_hresult().into());
        }
        unsafe {
            CfSetInSyncState(
                self.reference.handle(),
                CloudFilters::CF_IN_SYNC_STATE_IN_SYNC,
                CloudFilters::CF_SET_IN_SYNC_FLAG_NONE,
                None,
            )
        }?;
        generation_for_handle(self.reference.handle())
    }
}

fn generation_for_handle(handle: HANDLE) -> core::Result<FileGeneration> {
    let mut identity = FILE_ID_INFO::default();
    unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            ptr::from_mut(&mut identity).cast(),
            mem::size_of::<FILE_ID_INFO>() as u32,
        )
    }?;
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
            Some(ptr::from_ref(&input).cast()),
            mem::size_of_val(&input) as u32,
            Some(buffer.as_mut_ptr().cast()),
            mem::size_of_val(&buffer) as u32,
            Some(&mut returned),
            None,
        )
    }?;
    if (returned as usize) < mem::size_of::<USN_RECORD_V2>() {
        return Err(E_UNEXPECTED.into());
    }
    let record = unsafe { &*buffer.as_ptr().cast::<USN_RECORD_V2>() };
    if record.MajorVersion != 2 || record.Usn <= 0 {
        return Err(E_UNEXPECTED.into());
    }
    Ok(FileGeneration {
        volume: identity.VolumeSerialNumber,
        file_id: identity.FileId.Identifier,
        usn: record.Usn,
    })
}

impl Placeholder {
    /// Observe the handle's identity and nonzero NTFS USN. The caller must bind
    /// this observation to its qualified immutable snapshot, not a timer/size.
    pub fn generation(&self) -> core::Result<FileGeneration> {
        self.with_win32_handle(generation_for_handle)
    }

    /// Acknowledge an already verified uploaded generation using a short exclusive
    /// RH oplock barrier. Conflicting opens wait until the reference is released;
    /// existing writers/writable mappings prevent acquisition. A replaced file or
    /// stale USN fails before changing state. Do not hold this barrier over upload.
    ///
    /// Unlike the raw conditional `mark_in_sync`, this does not rely on CFAPI's
    /// nonzero-USN transition on a dirty placeholder. The native state change has
    /// no USN argument only after validating the nonzero identity/USN under the
    /// retained exclusive reference. Any acquisition/query/state error is returned.
    pub fn acknowledge_uploaded(
        path: impl AsRef<Path>,
        expected: FileGeneration,
    ) -> core::Result<FileGeneration> {
        UploadAcknowledgement::begin(path, expected)?.commit()
    }
    /// Create a placeholder from a raw handle.
    ///
    /// # Safety
    ///
    /// The passed handle must be a valid protected handle or win32 handle.
    pub unsafe fn from_raw_handle(handle: OwnedPlaceholderHandle) -> Self {
        Self {
            handle: Arc::new(handle),
        }
    }

    /// Open options for opening [Placeholder]s.
    pub fn options() -> OpenOptions {
        OpenOptions::default()
    }

    /// Open the placeholder file/directory with `CF_OPEN_FILE_FLAG_NONE`.
    pub fn open(path: impl AsRef<Path>) -> core::Result<Self> {
        OpenOptions::new().open(path)
    }

    // Keep the native handle referenced until the synchronous operation returns.
    // CfOpenFileWithOplock returns an opaque protected handle, not a Win32 HANDLE.
    fn with_win32_handle<T>(
        &self,
        call: impl FnOnce(HANDLE) -> core::Result<T>,
    ) -> core::Result<T> {
        match self.handle.handle_type {
            PlaceholderHandleType::Win32 => call(self.handle.handle),
            PlaceholderHandleType::CfApi => {
                let reference = self.win32_handle()?;
                call(reference.handle())
            }
        }
    }

    /// Marks a placeholder as in sync or not.
    ///
    /// If the passed [Usn] is outdated, the call will fail,
    /// otherwise the [Usn] will be updated.
    ///
    /// See also
    /// [SetInSyncState](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfsetinsyncstate),
    /// [What does "In-Sync" Mean?](https://www.userfilesystem.com/programming/faq/#nav_whatdoesin-syncmean)
    pub fn mark_in_sync<'a>(
        &mut self,
        in_sync: bool,
        usn: impl Into<Option<&'a mut Usn>>,
    ) -> core::Result<&mut Self> {
        self.with_win32_handle(|handle| unsafe {
            CfSetInSyncState(
                handle,
                match in_sync {
                    true => CloudFilters::CF_IN_SYNC_STATE_IN_SYNC,
                    false => CloudFilters::CF_IN_SYNC_STATE_NOT_IN_SYNC,
                },
                CloudFilters::CF_SET_IN_SYNC_FLAG_NONE,
                usn.into().map(ptr::from_mut),
            )
        })?;

        Ok(self)
    }

    /// Sets the pin state of the placeholder.
    ///
    /// See also
    /// [CfSetPinState](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfsetpinstate),
    /// [What does "Pinned" Mean?](https://www.userfilesystem.com/programming/faq/#nav_howdoesthealwayskeeponthisdevicemenuworks)
    pub fn mark_pin(&mut self, state: PinState, options: PinOptions) -> core::Result<&mut Self> {
        self.with_win32_handle(|handle| unsafe {
            CfSetPinState(handle, state.into(), options.0, None)
        })?;
        Ok(self)
    }

    /// Converts a file to a placeholder file.
    ///
    /// The optional [Usn] receives the final USN after conversion. Its input
    /// value is ignored; unlike [Self::mark_in_sync], this is not a conditional
    /// update against an expected USN.
    ///
    /// See also [CfConvertToPlaceholder](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfconverttoplaceholder).
    pub fn convert_to_placeholder<'a>(
        &mut self,
        options: ConvertOptions,
        usn: impl Into<Option<&'a mut Usn>>,
    ) -> core::Result<&mut Self> {
        self.with_win32_handle(|handle| unsafe {
            CfConvertToPlaceholder(
                handle,
                (!options.blob.is_empty()).then_some(options.blob.as_ptr() as *const _),
                options.blob.len() as _,
                options.flags,
                usn.into().map(ptr::from_mut),
                None,
            )
        })?;

        Ok(self)
    }

    /// Gets various characteristics of the placeholder.
    ///
    /// Returns [None] if the handle not points to a placeholder.
    ///
    /// If the placeholder blob size is known, use [fixed_size_info](Self::fixed_size_info) instead.
    pub fn info(&self) -> core::Result<Option<PlaceholderInfo>> {
        self.fixed_size_info(CloudFilters::CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH as usize)
    }

    /// Queries placeholder information with capacity for `blob_size` identity bytes.
    /// Returns native insufficient-buffer errors; no partially initialized result.
    pub fn fixed_size_info(&self, blob_size: usize) -> core::Result<Option<PlaceholderInfo>> {
        if blob_size > CloudFilters::CF_PLACEHOLDER_MAX_FILE_IDENTITY_LENGTH as usize {
            return Err(E_INVALIDARG.into());
        }
        let capacity = mem::size_of::<CF_PLACEHOLDER_STANDARD_INFO>() + blob_size;
        // The native structure requires 8-byte alignment. Own header/identity data
        // independently so cloning never retains a pointer into another Vec.
        let mut data = vec![0u64; capacity.div_ceil(mem::size_of::<u64>())];
        let mut returned = 0u32;
        let result = self.with_win32_handle(|handle| unsafe {
            CfGetPlaceholderInfo(
                handle,
                CloudFilters::CF_PLACEHOLDER_INFO_STANDARD,
                data.as_mut_ptr().cast(),
                capacity as u32,
                Some(&mut returned),
            )
        });
        match result {
            Ok(()) => {
                let info =
                    unsafe { ptr::read(data.as_ptr().cast::<CF_PLACEHOLDER_STANDARD_INFO>()) };
                let offset = mem::offset_of!(CF_PLACEHOLDER_STANDARD_INFO, FileIdentity);
                let length = info.FileIdentityLength as usize;
                if (returned as usize) < offset
                    || (returned as usize) > capacity
                    || length > (returned as usize) - offset
                {
                    return Err(E_UNEXPECTED.into());
                }
                let blob = unsafe {
                    std::slice::from_raw_parts(data.as_ptr().cast::<u8>().add(offset), length)
                }
                .to_vec();
                Ok(Some(PlaceholderInfo { info, blob }))
            }
            Err(e) if e.code() == ERROR_NOT_A_CLOUD_FILE.to_hresult() => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Updates various characteristics of a placeholder.
    ///
    /// See also [CfUpdatePlaceholder](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfupdateplaceholder).
    pub fn update<'a>(
        &mut self,
        options: UpdateOptions,
        usn: impl Into<Option<&'a mut Usn>>,
    ) -> core::Result<&mut Self> {
        self.with_win32_handle(|handle| unsafe {
            CfUpdatePlaceholder(
                handle,
                options.metadata.as_ref().map(|x| ptr::from_ref(&x.0)),
                (!options.blob.is_empty()).then_some(options.blob.as_ptr() as *const _),
                options.blob.len() as _,
                (!options.dehydrate_ranges.is_empty())
                    .then_some(options.dehydrate_ranges.as_slice()),
                options.flags,
                usn.into().map(ptr::from_mut),
                None,
            )
        })?;

        Ok(self)
    }

    /// Retrieves serialized CF_FILE_RANGE records, not file content.
    pub fn retrieve_data(
        &self,
        read_type: ReadType,
        offset: u64,
        buffer: &mut [u8],
    ) -> core::Result<u32> {
        let offset = i64::try_from(offset).map_err(|_| core::Error::from(E_INVALIDARG))?;
        let size = u32::try_from(buffer.len()).map_err(|_| core::Error::from(E_INVALIDARG))?;
        let mut length = 0;
        self.with_win32_handle(|handle| unsafe {
            CfGetPlaceholderRangeInfo(
                handle,
                read_type.into(),
                offset,
                buffer.len() as i64,
                buffer.as_mut_ptr().cast(),
                size,
                Some(&mut length),
            )
        })?;
        Ok(length)
    }

    // FIXME: This function is not work at all, the CF_PLACEHOLDER_STATE always be 0 or 1
    // pub fn state(&self) -> core::Result<Option<PlaceholderState>> {
    //     let mut info = MaybeUninit::<FILE_ATTRIBUTE_TAG_INFO>::zeroed();
    //     let win32_handle = self.win32_handle()?;
    //     let state = unsafe {
    //         GetFileInformationByHandleEx(
    //             win32_handle.win32_handle,
    //             FileSystem::FileAttributeTagInfo,
    //             info.as_mut_ptr() as *mut _,
    //             mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
    //         )
    //         .ok()
    //         .inspect_err(|e| println!("GetFileInformationByHandleEx: {e:#?}"))?;

    //         CfGetPlaceholderStateFromFileInfo(
    //             info.assume_init_ref() as *const _ as *const _,
    //             FileSystem::FileAttributeTagInfo,
    //         )
    //     };

    //     match state {
    //         CloudFilters::CF_PLACEHOLDER_STATE_INVALID => Err(core::Error::from_win32()),
    //         CloudFilters::CF_PLACEHOLDER_STATE_NO_STATES => Ok(None),
    //         s => Ok(Some(PlaceholderState(s))),
    //     }
    // }

    /// Returns the Win32 handle from protected handle.
    ///
    /// Returns `Err(E_HANDLE)` if the [OwnedPlaceholderHandle::handle_type] is not [PlaceholderHandleType::CfApi].
    pub fn win32_handle(&self) -> core::Result<ArcWin32Handle> {
        if self.handle.handle_type != PlaceholderHandleType::CfApi {
            return Err(E_HANDLE.into());
        }
        if !unsafe { CfReferenceProtectedHandle(self.handle.handle) } {
            // CfReferenceProtectedHandle documents only BOOLEAN, not last-error.
            return Err(E_HANDLE.into());
        }
        let win32_handle = unsafe { CfGetWin32HandleFromProtectedHandle(self.handle.handle) };
        if win32_handle.is_invalid() {
            unsafe { CfReleaseProtectedHandle(self.handle.handle) };
            return Err(E_HANDLE.into());
        }
        Ok(ArcWin32Handle {
            inner: Arc::new(ProtectedReference {
                win32_handle,
                owner: Arc::clone(&self.handle),
            }),
        })
    }

    /// Returns the owned placeholder handle.
    pub fn inner_handle(&self) -> &OwnedPlaceholderHandle {
        &self.handle
    }

    /// Hydrates a placeholder file by ensuring that the specified byte range is present on-disk
    /// in the placeholder. This is valid for files only.
    ///
    /// Invalid/reversed/overflowing ranges return E_INVALIDARG.
    pub fn hydrate(&mut self, range: impl RangeBounds<u64>) -> core::Result<()> {
        let (offset, length) = hydration_range(range)?;
        if length == 0 {
            return Ok(());
        }
        self.with_win32_handle(|handle| unsafe {
            CfHydratePlaceholder(
                handle,
                offset,
                length,
                CloudFilters::CF_HYDRATE_FLAG_NONE,
                None,
            )
        })
    }
}

fn hydration_range(range: impl RangeBounds<u64>) -> core::Result<(i64, i64)> {
    let invalid = || core::Error::from(E_INVALIDARG);
    let start = match range.start_bound() {
        Bound::Included(x) => *x,
        Bound::Excluded(x) => x.checked_add(1).ok_or_else(invalid)?,
        Bound::Unbounded => 0,
    };
    let offset = i64::try_from(start).map_err(|_| invalid())?;
    let end = match range.end_bound() {
        Bound::Included(x) => Some(x.checked_add(1).ok_or_else(invalid)?),
        Bound::Excluded(x) => Some(*x),
        Bound::Unbounded => None,
    };
    let length = match end {
        Some(end) => {
            i64::try_from(end.checked_sub(start).ok_or_else(invalid)?).map_err(|_| invalid())?
        }
        None => -1,
    };
    Ok((offset, length))
}

impl From<File> for Placeholder {
    fn from(file: File) -> Self {
        Self {
            handle: Arc::new(unsafe {
                OwnedPlaceholderHandle::from_win32(HANDLE(file.into_raw_handle() as _))
            }),
        }
    }
}

impl TryFrom<Placeholder> for File {
    type Error = core::Error;

    /// Transfers an ordinary synchronous Win32 handle without closing it twice.
    /// Protected handles are rejected: they are asynchronous and cannot safely
    /// become std::fs::File; use a short win32_handle guard with overlapped I/O.
    /// This conversion never reverts or dehydrates the placeholder.
    fn try_from(placeholder: Placeholder) -> core::Result<Self> {
        if placeholder.handle.handle_type != PlaceholderHandleType::Win32 {
            return Err(E_INVALIDARG.into());
        }
        let owner = Arc::try_unwrap(placeholder.handle).map_err(|_| core::Error::from(E_HANDLE))?;
        let owner = ManuallyDrop::new(owner);
        Ok(unsafe { File::from_raw_handle(owner.handle.0) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hydration_ranges_use_length_and_reject_overflow() {
        assert_eq!(hydration_range(5..9).unwrap(), (5, 4));
        assert_eq!(hydration_range(5..=9).unwrap(), (5, 5));
        assert_eq!(hydration_range(5..).unwrap(), (5, -1));
        assert_eq!(hydration_range(0..0).unwrap(), (0, 0));
        assert!(hydration_range((Bound::Excluded(u64::MAX), Bound::Unbounded)).is_err());
        assert!(hydration_range((Bound::Included(9), Bound::Excluded(5))).is_err());
        assert!(hydration_range(..=u64::MAX).is_err());
        assert!(FileGeneration::from_parts(1, [1; 16], 0).is_err());
        assert!(FileGeneration::from_parts(1, [1; 16], -1).is_err());
    }
}

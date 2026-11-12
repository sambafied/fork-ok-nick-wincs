use std::ops::Range;

use windows::{
    core,
    Win32::{
        Foundation::E_INVALIDARG,
        Storage::CloudFilters::{CfReportProviderProgress, CF_CONNECTION_KEY},
    },
};

use crate::{
    command::{self, Command, Fallible},
    error::CloudErrorKind,
    filter::{RawConnectionKey, RawTransferKey},
    placeholder_file::PlaceholderFile,
    sealed, utility,
};

/// A ticket for the [SyncFilter::fetch_data][crate::filter::SyncFilter::fetch_data] callback.
#[derive(Debug)]
pub struct FetchData {
    connection_key: RawConnectionKey,
    transfer_key: RawTransferKey,
}

impl FetchData {
    /// Create a new [FetchData].
    pub(crate) fn new(connection_key: RawConnectionKey, transfer_key: RawTransferKey) -> Self {
        Self {
            connection_key,
            transfer_key,
        }
    }

    /// Completes a failed hydration over its original nonempty required range.
    ///
    /// This consumes the ticket. A deferred worker must retain the original
    /// connection and its registration owners until this call has returned;
    /// retaining the numeric ticket alone does not keep the connection alive.
    /// Invalid or unrepresentable ranges fail before issuing a native operation.
    pub fn fail_range(self, error_kind: CloudErrorKind, range: Range<u64>) -> core::Result<()> {
        let (offset, length) = failure_range(range)?;
        command::Write::fail_range(
            self.connection_key,
            self.transfer_key,
            error_kind,
            offset,
            length,
        )
    }

    /// Displays a progress bar next to the file in the file explorer to show the progress of the
    /// current operation. In addition, the standard Windows file progress dialog will open
    /// displaying the speed and progress based on the values set. During background hydrations,
    /// an interactive toast will appear notifying the user of an operation with a progress bar.
    pub fn report_progress(&self, total: u64, completed: u64) -> core::Result<()> {
        unsafe {
            CfReportProviderProgress(
                CF_CONNECTION_KEY(self.connection_key),
                self.transfer_key,
                total as i64,
                completed as i64,
            )
        }?;

        Ok(())
    }

    // TODO: response command::Update
}

fn failure_range(range: Range<u64>) -> core::Result<(i64, i64)> {
    let invalid = || core::Error::from_hresult(E_INVALIDARG);
    // Validate the endpoint too: two representable operands must not describe
    // a native range whose exclusive end exceeds the signed CFAPI domain.
    i64::try_from(range.end).map_err(|_| invalid())?;
    let offset = i64::try_from(range.start).map_err(|_| invalid())?;
    let length = range
        .end
        .checked_sub(range.start)
        .filter(|length| *length != 0)
        .ok_or_else(invalid)?;
    let length = i64::try_from(length).map_err(|_| invalid())?;
    Ok((offset, length))
}

impl utility::ReadAt for FetchData {
    /// Read data at an offset from a placeholder file.
    ///
    /// This method is equivalent to calling `CfExecute` with `CF_OPERATION_TYPE_RETRIEVE_DATA`.
    fn read_at(&self, buf: &mut [u8], offset: u64) -> core::Result<u64> {
        command::Read {
            buffer: buf,
            position: offset,
        }
        .execute(self.connection_key, self.transfer_key)
    }
}

impl utility::WriteAt for FetchData {
    /// Write data at an offset to a placeholder file.
    ///
    /// The buffer passed must be 4KiB in length or end on the logical file size. Unfortunately,
    /// this is a restriction of the operating system.
    ///
    /// This method is equivalent to calling `CfExecute` with `CF_OPERATION_TYPE_TRANSFER_DATA`.
    fn write_at(&self, buf: &[u8], offset: u64) -> core::Result<()> {
        command::Write {
            buffer: buf,
            position: offset,
        }
        .execute(self.connection_key, self.transfer_key)
    }
}

impl sealed::Sealed for FetchData {}

/// A ticket for the [SyncFilter::validate_data][crate::filter::SyncFilter::validate_data] callback.
#[derive(Debug)]
pub struct ValidateData {
    connection_key: RawConnectionKey,
    transfer_key: RawTransferKey,
}

impl ValidateData {
    /// Create a new [ValidateData].
    pub(crate) fn new(connection_key: RawConnectionKey, transfer_key: RawTransferKey) -> Self {
        Self {
            connection_key,
            transfer_key,
        }
    }

    /// Validates the data range in the placeholder file is valid.
    ///
    /// This method is equivalent to calling `CfExecute` with `CF_OPERATION_TYPE_ACK_DATA`.
    // TODO: make this generic over a RangeBounds
    // if the range specified is past the current file length, will it consider that range to be validated?
    // https://docs.microsoft.com/en-us/answers/questions/750302/if-the-ackdata-field-of-cf-operation-parameters-is.html
    pub fn pass(&self, range: Range<u64>) -> core::Result<()> {
        command::Validate { range }.execute(self.connection_key, self.transfer_key)
    }

    // TODO: response command::Update
}

impl utility::ReadAt for ValidateData {
    /// Read data at an offset from a placeholder file.
    ///
    /// This method is equivalent to calling `CfExecute` with `CF_OPERATION_TYPE_RETRIEVE_DATA`.
    ///
    /// The bytes returned will ALWAYS be the length of the buffer passed in. The operating
    /// system provides these guarantees.
    fn read_at(&self, buf: &mut [u8], offset: u64) -> core::Result<u64> {
        command::Read {
            buffer: buf,
            position: offset,
        }
        .execute(self.connection_key, self.transfer_key)
    }
}

impl sealed::Sealed for ValidateData {}

/// A ticket for the [SyncFilter::fetch_placeholders][crate::filter::SyncFilter::fetch_placeholders] callback.
#[derive(Debug)]
pub struct FetchPlaceholders {
    connection_key: RawConnectionKey,
    transfer_key: RawTransferKey,
}

impl FetchPlaceholders {
    /// Create a new [FetchPlaceholders].
    pub(crate) fn new(connection_key: RawConnectionKey, transfer_key: RawTransferKey) -> Self {
        Self {
            connection_key,
            transfer_key,
        }
    }

    /// Creates a complete directory listing in one batch.
    /// Inspect each placeholder's result after the call for per-entry failures.
    pub fn pass_with_placeholder(&self, placeholders: &mut [PlaceholderFile]) -> core::Result<()> {
        let count = placeholders.len() as u64;
        self.pass_batch(placeholders, count, true).map(|_| ())
    }

    /// Transfers a page and returns the number of entries processed by Windows.
    /// `total` is the full directory entry count, not the size of this page.
    /// Set `final_batch` only after all remote pages have been obtained successfully;
    /// only that call marks population complete. Inspect per-entry results too.
    /// An empty final page completes an empty directory or a prior page sequence.
    pub fn pass_batch(
        &self,
        placeholders: &mut [PlaceholderFile],
        total: u64,
        final_batch: bool,
    ) -> core::Result<u32> {
        command::placeholder_batch(
            placeholders,
            total,
            final_batch,
            self.connection_key,
            self.transfer_key,
        )
    }
}

/// A ticket for the [SyncFilter::dehydrate][crate::filter::SyncFilter::dehydrate] callback.
#[derive(Debug)]
pub struct Dehydrate {
    connection_key: RawConnectionKey,
    transfer_key: RawTransferKey,
}

impl Dehydrate {
    /// Create a new [Dehydrate].
    pub(crate) fn new(connection_key: RawConnectionKey, transfer_key: RawTransferKey) -> Self {
        Self {
            connection_key,
            transfer_key,
        }
    }

    /// Refuses the original dehydration request, consuming its completion ticket.
    /// A deferred worker must retain the original connection and registration
    /// owners through the returned native result, including failure/uncertainty.
    pub fn fail(self, error_kind: CloudErrorKind) -> core::Result<()> {
        command::Dehydrate::fail(self.connection_key, self.transfer_key, error_kind)
    }

    /// Confirms dehydration of the file.
    pub fn pass(&self) -> core::Result<()> {
        command::Dehydrate { blob: &[] }.execute(self.connection_key, self.transfer_key)
    }

    /// Confirms dehydration of the file and updates its file blob.
    pub fn pass_with_blob(&self, blob: &[u8]) -> core::Result<()> {
        command::Dehydrate { blob }.execute(self.connection_key, self.transfer_key)
    }
}

/// A ticket for the [SyncFilter::delete][crate::filter::SyncFilter::delete] callback.
#[derive(Debug)]
pub struct Delete {
    connection_key: RawConnectionKey,
    transfer_key: RawTransferKey,
}

impl Delete {
    /// Create a new [Delete].
    pub(crate) fn new(connection_key: RawConnectionKey, transfer_key: RawTransferKey) -> Self {
        Self {
            connection_key,
            transfer_key,
        }
    }

    /// Confirms deletion of the file.
    pub fn pass(&self) -> core::Result<()> {
        command::Delete.execute(self.connection_key, self.transfer_key)
    }
}

/// A ticket for the [SyncFilter::rename][crate::filter::SyncFilter::rename] callback.
#[derive(Debug)]
pub struct Rename {
    connection_key: RawConnectionKey,
    transfer_key: RawTransferKey,
}

impl Rename {
    /// Create a new [Rename].
    pub(crate) fn new(connection_key: RawConnectionKey, transfer_key: RawTransferKey) -> Self {
        Self {
            connection_key,
            transfer_key,
        }
    }

    /// Confirms the rename/move of a file.
    pub fn pass(&self) -> core::Result<()> {
        command::Rename.execute(self.connection_key, self.transfer_key)
    }
}

#[cfg(test)]
mod failure_range_tests {
    use super::*;

    #[test]
    fn accepts_zero_offset_and_single_byte_eof_tail() {
        assert_eq!(failure_range(0..4096).unwrap(), (0, 4096));
        assert_eq!(failure_range(8192..8193).unwrap(), (8192, 1));
    }

    #[test]
    fn accepts_the_largest_signed_exclusive_endpoint() {
        let end = i64::MAX as u64;
        assert_eq!(failure_range(0..end).unwrap(), (0, i64::MAX));
        assert_eq!(failure_range(end - 1..end).unwrap(), (i64::MAX - 1, 1));
    }

    #[test]
    fn rejects_empty_reversed_and_unsigned_ranges() {
        let maximum = i64::MAX as u64;
        for range in [
            0..0,
            4..4,
            Range { start: 5, end: 4 },
            0..maximum + 1,
            maximum..u64::MAX,
            Range {
                start: u64::MAX,
                end: 0,
            },
            u64::MAX..u64::MAX,
        ] {
            assert_eq!(failure_range(range).unwrap_err().code(), E_INVALIDARG);
        }
    }
}

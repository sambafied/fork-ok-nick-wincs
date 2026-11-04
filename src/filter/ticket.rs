use std::ops::Range;

use windows::{
    core,
    Win32::Storage::CloudFilters::{CfReportProviderProgress, CF_CONNECTION_KEY},
};

use crate::{
    command::{self, Command},
    filter::OperationContext,
    placeholder_file::PlaceholderFile,
    sealed, utility,
};

/// A ticket for the [SyncFilter::fetch_data][crate::filter::SyncFilter::fetch_data] callback.
#[derive(Debug)]
pub struct FetchData {
    context: OperationContext,
}

impl FetchData {
    /// Create a new [FetchData].
    pub(crate) fn new(context: OperationContext) -> Self {
        Self { context }
    }

    /// The owned operation context captured for this original callback.
    pub fn operation_context(&self) -> &OperationContext {
        &self.context
    }

    /// Displays a progress bar next to the file in the file explorer to show the progress of the
    /// current operation. In addition, the standard Windows file progress dialog will open
    /// displaying the speed and progress based on the values set. During background hydrations,
    /// an interactive toast will appear notifying the user of an operation with a progress bar.
    pub fn report_progress(&self, total: u64, completed: u64) -> core::Result<()> {
        unsafe {
            CfReportProviderProgress(
                CF_CONNECTION_KEY(self.context.connection_key()),
                self.context.transfer_key(),
                total as i64,
                completed as i64,
            )
        }?;

        Ok(())
    }

    // TODO: response command::Update
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
        .execute_with_context(&self.context)
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
        .execute_with_context(&self.context)
    }
}

impl sealed::Sealed for FetchData {}

/// A ticket for the [SyncFilter::validate_data][crate::filter::SyncFilter::validate_data] callback.
#[derive(Debug)]
pub struct ValidateData {
    context: OperationContext,
}

impl ValidateData {
    /// Create a new [ValidateData].
    pub(crate) fn new(context: OperationContext) -> Self {
        Self { context }
    }

    /// The owned operation context captured for this original callback.
    pub fn operation_context(&self) -> &OperationContext {
        &self.context
    }

    /// Validates the data range in the placeholder file is valid.
    ///
    /// This method is equivalent to calling `CfExecute` with `CF_OPERATION_TYPE_ACK_DATA`.
    // TODO: make this generic over a RangeBounds
    // if the range specified is past the current file length, will it consider that range to be validated?
    // https://docs.microsoft.com/en-us/answers/questions/750302/if-the-ackdata-field-of-cf-operation-parameters-is.html
    pub fn pass(&self, range: Range<u64>) -> core::Result<()> {
        command::Validate { range }.execute_with_context(&self.context)
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
        .execute_with_context(&self.context)
    }
}

impl sealed::Sealed for ValidateData {}

/// A ticket for the [SyncFilter::fetch_placeholders][crate::filter::SyncFilter::fetch_placeholders] callback.
#[derive(Debug)]
pub struct FetchPlaceholders {
    context: OperationContext,
}

impl FetchPlaceholders {
    /// Create a new [FetchPlaceholders].
    pub(crate) fn new(context: OperationContext) -> Self {
        Self { context }
    }

    /// The owned operation context captured for this original callback.
    pub fn operation_context(&self) -> &OperationContext {
        &self.context
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
        command::placeholder_batch(placeholders, total, final_batch, &self.context)
    }
}

/// A ticket for the [SyncFilter::dehydrate][crate::filter::SyncFilter::dehydrate] callback.
#[derive(Debug)]
pub struct Dehydrate {
    context: OperationContext,
}

impl Dehydrate {
    /// Create a new [Dehydrate].
    pub(crate) fn new(context: OperationContext) -> Self {
        Self { context }
    }

    /// The owned operation context captured for this original callback.
    pub fn operation_context(&self) -> &OperationContext {
        &self.context
    }

    /// Confirms dehydration of the file.
    pub fn pass(&self) -> core::Result<()> {
        command::Dehydrate { blob: &[] }.execute_with_context(&self.context)
    }

    /// Confirms dehydration of the file and updates its file blob.
    pub fn pass_with_blob(&self, blob: &[u8]) -> core::Result<()> {
        command::Dehydrate { blob }.execute_with_context(&self.context)
    }
}

/// A ticket for the [SyncFilter::delete][crate::filter::SyncFilter::delete] callback.
#[derive(Debug)]
pub struct Delete {
    context: OperationContext,
}

impl Delete {
    /// Create a new [Delete].
    pub(crate) fn new(context: OperationContext) -> Self {
        Self { context }
    }

    /// The owned operation context captured for this original callback.
    pub fn operation_context(&self) -> &OperationContext {
        &self.context
    }

    /// Confirms deletion of the file.
    pub fn pass(&self) -> core::Result<()> {
        command::Delete.execute_with_context(&self.context)
    }
}

/// A ticket for the [SyncFilter::rename][crate::filter::SyncFilter::rename] callback.
#[derive(Debug)]
pub struct Rename {
    context: OperationContext,
}

impl Rename {
    /// Create a new [Rename].
    pub(crate) fn new(context: OperationContext) -> Self {
        Self { context }
    }

    /// The owned operation context captured for this original callback.
    pub fn operation_context(&self) -> &OperationContext {
        &self.context
    }

    /// Confirms the rename/move of a file.
    pub fn pass(&self) -> core::Result<()> {
        command::Rename.execute_with_context(&self.context)
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;
    use crate::filter::Request;
    use windows::Win32::{
        Storage::CloudFilters::{CF_CALLBACK_INFO, CF_OPERATION_TYPE_TRANSFER_DATA},
        System::CorrelationVector::CORRELATION_VECTOR,
    };

    #[test]
    fn every_ticket_retains_the_original_owned_callback_context() {
        let mut correlation = CORRELATION_VECTOR::default();
        correlation.Vector[0] = 42;
        let request = Request::new(CF_CALLBACK_INFO {
            ConnectionKey: CF_CONNECTION_KEY(7),
            TransferKey: 11,
            RequestKey: 17,
            CorrelationVector: &correlation,
            ..Default::default()
        });
        let context = request.operation_context();
        let fetch = FetchData::new(context.clone());
        let validate = ValidateData::new(context.clone());
        let populate = FetchPlaceholders::new(context.clone());
        let dehydrate = Dehydrate::new(context.clone());
        let delete = Delete::new(context.clone());
        let rename = Rename::new(context.clone());
        correlation.Vector[0] = 9;
        for retained in [
            fetch.operation_context(),
            validate.operation_context(),
            populate.operation_context(),
            dehydrate.operation_context(),
            delete.operation_context(),
            rename.operation_context(),
        ] {
            assert_eq!(retained.identity(), context.identity());
            let operation = retained.operation_info(CF_OPERATION_TYPE_TRANSFER_DATA);
            assert_eq!(operation.ConnectionKey.0, 7);
            assert_eq!(operation.TransferKey, 11);
            assert_eq!(operation.RequestKey, 17);
            assert_eq!(unsafe { (*operation.CorrelationVector).Vector[0] }, 42);
        }
    }
}

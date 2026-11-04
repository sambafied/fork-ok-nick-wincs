use std::{ops::Range, ptr};

use windows::{
    core,
    Win32::{
        Foundation,
        Storage::CloudFilters::{
            self, CF_OPERATION_PARAMETERS_0, CF_OPERATION_PARAMETERS_0_0,
            CF_OPERATION_PARAMETERS_0_1, CF_OPERATION_PARAMETERS_0_2, CF_OPERATION_PARAMETERS_0_3,
            CF_OPERATION_PARAMETERS_0_4, CF_OPERATION_PARAMETERS_0_5, CF_OPERATION_PARAMETERS_0_6,
            CF_OPERATION_PARAMETERS_0_7, CF_OPERATION_TYPE,
        },
    },
};

use crate::{
    command::executor::{execute_with_context, Command, Fallible},
    error::CloudErrorKind,
    filter::{OperationContext, RawConnectionKey, RawTransferKey},
    metadata::Metadata,
    placeholder_file::PlaceholderFile,
};

/// Read data from a placeholder file.
#[derive(Debug)]
pub struct Read<'a> {
    /// The buffer of data to read into.
    pub buffer: &'a mut [u8],
    /// The starting offset of from where to read.
    pub position: u64,
}

impl Command for Read<'_> {
    const OPERATION: CF_OPERATION_TYPE = CloudFilters::CF_OPERATION_TYPE_RETRIEVE_DATA;

    type Result = u64;
    type Field = CF_OPERATION_PARAMETERS_0_1;

    unsafe fn result(info: CF_OPERATION_PARAMETERS_0) -> Self::Result {
        info.RetrieveData.ReturnedLength as u64
    }

    fn build(&self) -> CF_OPERATION_PARAMETERS_0 {
        CF_OPERATION_PARAMETERS_0 {
            RetrieveData: CF_OPERATION_PARAMETERS_0_1 {
                Flags: CloudFilters::CF_OPERATION_RETRIEVE_DATA_FLAG_NONE,
                Buffer: self.buffer.as_ptr() as *mut _,
                Offset: self.position as i64,
                Length: self.buffer.len() as i64,
                ReturnedLength: 0,
            },
        }
    }
}

/// Write data to a placeholder file.
#[derive(Debug)]
pub struct Write<'a> {
    /// The buffer of data to write into the file.
    pub buffer: &'a [u8],
    /// The starting offset of from where to write.
    pub position: u64,
}

impl Command for Write<'_> {
    const OPERATION: CF_OPERATION_TYPE = CloudFilters::CF_OPERATION_TYPE_TRANSFER_DATA;

    type Result = ();
    type Field = CF_OPERATION_PARAMETERS_0_0;

    unsafe fn result(_info: CF_OPERATION_PARAMETERS_0) -> Self::Result {}

    fn build(&self) -> CF_OPERATION_PARAMETERS_0 {
        CF_OPERATION_PARAMETERS_0 {
            TransferData: CF_OPERATION_PARAMETERS_0_0 {
                // TODO: add flag for disable_on_demand_population
                Flags: CloudFilters::CF_OPERATION_TRANSFER_DATA_FLAG_NONE,
                CompletionStatus: Foundation::STATUS_SUCCESS,
                Buffer: self.buffer.as_ptr() as *mut _,
                Offset: self.position as i64,
                Length: self.buffer.len() as i64,
            },
        }
    }
}

impl Write<'_> {
    /// Complete failed user I/O over the requested range; a zero-length range
    /// overlaps no pending reads and cannot report their failure.
    pub(crate) fn fail_range(
        context: &OperationContext,
        error_kind: CloudErrorKind,
        offset: i64,
        length: i64,
    ) -> core::Result<()> {
        execute_with_context::<Self>(
            Self::failure_parameters(error_kind, offset, length),
            context,
        )
    }

    fn failure_parameters(
        error_kind: CloudErrorKind,
        offset: i64,
        length: i64,
    ) -> CF_OPERATION_PARAMETERS_0 {
        CF_OPERATION_PARAMETERS_0 {
            TransferData: CF_OPERATION_PARAMETERS_0_0 {
                Flags: CloudFilters::CF_OPERATION_TRANSFER_DATA_FLAG_NONE,
                CompletionStatus: error_kind.into(),
                Buffer: std::ptr::null_mut(),
                Offset: offset,
                Length: length,
            },
        }
    }
}

/// Update various properties on a placeholder.
#[derive(Debug)]
#[allow(dead_code)]
pub struct Update<'a> {
    /// Whether or not to mark the placeholder as "synced."
    pub mark_in_sync: bool,
    /// Optional metadata to update.
    pub metadata: Option<Metadata>,
    /// Optional file blob to update.
    pub blob: &'a [u8],
}

impl Command for Update<'_> {
    const OPERATION: CF_OPERATION_TYPE = CloudFilters::CF_OPERATION_TYPE_RESTART_HYDRATION;

    type Result = ();
    type Field = CF_OPERATION_PARAMETERS_0_3;

    unsafe fn result(_info: CF_OPERATION_PARAMETERS_0) -> Self::Result {}

    fn build(&self) -> CF_OPERATION_PARAMETERS_0 {
        CF_OPERATION_PARAMETERS_0 {
            RestartHydration: CF_OPERATION_PARAMETERS_0_3 {
                Flags: if self.mark_in_sync {
                    CloudFilters::CF_OPERATION_RESTART_HYDRATION_FLAG_MARK_IN_SYNC
                } else {
                    CloudFilters::CF_OPERATION_RESTART_HYDRATION_FLAG_NONE
                },
                FsMetadata: self
                    .metadata
                    .as_ref()
                    .map_or(ptr::null(), |metadata| &metadata.0 as *const _),
                FileIdentity: match self.blob.is_empty() {
                    true => ptr::null(),
                    false => self.blob.as_ptr() as *const _,
                },
                FileIdentityLength: self.blob.len() as _,
            },
        }
    }
}

/// Create placeholder files/directories.
#[derive(Debug)]
pub struct CreatePlaceholders<'a> {
    /// The placeholders to create.
    pub placeholders: &'a mut [PlaceholderFile],
    /// The total amount of placeholders that are a child of the current directory.
    pub total: u64,
}

impl Command for CreatePlaceholders<'_> {
    const OPERATION: CF_OPERATION_TYPE = CloudFilters::CF_OPERATION_TYPE_TRANSFER_PLACEHOLDERS;

    type Result = ();
    type Field = CF_OPERATION_PARAMETERS_0_4;

    unsafe fn result(_info: CF_OPERATION_PARAMETERS_0) -> Self::Result {}

    fn build(&self) -> CF_OPERATION_PARAMETERS_0 {
        CF_OPERATION_PARAMETERS_0 {
            TransferPlaceholders: CF_OPERATION_PARAMETERS_0_4 {
                // TODO: this flag tells the system there are no more placeholders in this directory (when that can be untrue)
                //       in the future, implement streaming
                Flags: CloudFilters::CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAG_DISABLE_ON_DEMAND_POPULATION,
                CompletionStatus: Foundation::STATUS_SUCCESS,
                PlaceholderTotalCount: self.total as i64,
                PlaceholderArray: match self.placeholders.is_empty() {
                    // If the slice is empty there will be an invalid memory access error, not with null pointers, however.
                    true => ptr::null_mut(),
                    false => self.placeholders.as_ptr() as *mut _,
                },
                PlaceholderCount: self.placeholders.len() as u32,
                EntriesProcessed: 0,
            },
        }
    }
}

impl Fallible for CreatePlaceholders<'_> {
    fn fail(
        connection_key: RawConnectionKey,
        transfer_key: RawTransferKey,
        error_kind: CloudErrorKind,
    ) -> core::Result<Self::Result> {
        Self::fail_with_context(
            &OperationContext::from_keys(connection_key, transfer_key),
            error_kind,
        )
    }
}

impl CreatePlaceholders<'_> {
    /// Report callback failure using the original owned operation context.
    pub fn fail_with_context(
        context: &OperationContext,
        error_kind: CloudErrorKind,
    ) -> core::Result<<Self as Command>::Result> {
        execute_with_context::<Self>(Self::failure_parameters(error_kind), context)
    }

    fn failure_parameters(error_kind: CloudErrorKind) -> CF_OPERATION_PARAMETERS_0 {
        CF_OPERATION_PARAMETERS_0 {
            TransferPlaceholders: CF_OPERATION_PARAMETERS_0_4 {
                Flags: CloudFilters::CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAG_NONE,
                CompletionStatus: error_kind.into(),
                PlaceholderTotalCount: 0,
                // TODO: DOES THIS HAVE TO BE A VALID ARRAY?
                PlaceholderArray: ptr::null_mut(),
                PlaceholderCount: 0,
                EntriesProcessed: 0,
            },
        }
    }
}

/// Validate the data range in the placeholder file is valid.
#[derive(Debug)]
pub struct Validate {
    /// The range of data to validate as "good."
    pub range: Range<u64>,
}

impl Command for Validate {
    const OPERATION: CF_OPERATION_TYPE = CloudFilters::CF_OPERATION_TYPE_ACK_DATA;

    type Result = ();
    type Field = CF_OPERATION_PARAMETERS_0_2;

    unsafe fn result(_info: CF_OPERATION_PARAMETERS_0) -> Self::Result {}

    fn build(&self) -> CF_OPERATION_PARAMETERS_0 {
        CF_OPERATION_PARAMETERS_0 {
            AckData: CF_OPERATION_PARAMETERS_0_2 {
                Flags: CloudFilters::CF_OPERATION_ACK_DATA_FLAG_NONE,
                CompletionStatus: Foundation::STATUS_SUCCESS,
                Offset: self.range.start as i64,
                Length: self.range.end as i64,
            },
        }
    }
}

impl Fallible for Validate {
    fn fail(
        connection_key: RawConnectionKey,
        transfer_key: RawTransferKey,
        error_kind: CloudErrorKind,
    ) -> core::Result<Self::Result> {
        Self::fail_with_context(
            &OperationContext::from_keys(connection_key, transfer_key),
            error_kind,
        )
    }
}

impl Validate {
    /// Report callback failure using the original owned operation context.
    pub fn fail_with_context(
        context: &OperationContext,
        error_kind: CloudErrorKind,
    ) -> core::Result<<Self as Command>::Result> {
        execute_with_context::<Self>(Self::failure_parameters(error_kind), context)
    }

    fn failure_parameters(error_kind: CloudErrorKind) -> CF_OPERATION_PARAMETERS_0 {
        CF_OPERATION_PARAMETERS_0 {
            AckData: CF_OPERATION_PARAMETERS_0_2 {
                Flags: CloudFilters::CF_OPERATION_ACK_DATA_FLAG_NONE,
                CompletionStatus: error_kind.into(),
                Offset: 0,
                Length: 0,
            },
        }
    }
}

/// Confirm dehydration of the placeholder file and optionally update its blob.
#[derive(Debug)]
pub struct Dehydrate<'a> {
    /// Optional file blob to update.
    pub blob: &'a [u8],
}

impl Command for Dehydrate<'_> {
    const OPERATION: CF_OPERATION_TYPE = CloudFilters::CF_OPERATION_TYPE_ACK_DEHYDRATE;

    type Result = ();
    type Field = CF_OPERATION_PARAMETERS_0_5;

    unsafe fn result(_info: CF_OPERATION_PARAMETERS_0) -> Self::Result {}

    fn build(&self) -> CF_OPERATION_PARAMETERS_0 {
        CF_OPERATION_PARAMETERS_0 {
            AckDehydrate: CF_OPERATION_PARAMETERS_0_5 {
                Flags: CloudFilters::CF_OPERATION_ACK_DEHYDRATE_FLAG_NONE,
                CompletionStatus: Foundation::STATUS_SUCCESS,
                FileIdentity: match self.blob.is_empty() {
                    true => ptr::null(),
                    false => self.blob.as_ptr() as *const _,
                },
                FileIdentityLength: self.blob.len() as u32,
            },
        }
    }
}

impl Fallible for Dehydrate<'_> {
    fn fail(
        connection_key: RawConnectionKey,
        transfer_key: RawTransferKey,
        error_kind: CloudErrorKind,
    ) -> core::Result<Self::Result> {
        Self::fail_with_context(
            &OperationContext::from_keys(connection_key, transfer_key),
            error_kind,
        )
    }
}

impl Dehydrate<'_> {
    /// Report callback failure using the original owned operation context.
    pub fn fail_with_context(
        context: &OperationContext,
        error_kind: CloudErrorKind,
    ) -> core::Result<<Self as Command>::Result> {
        execute_with_context::<Self>(Self::failure_parameters(error_kind), context)
    }

    fn failure_parameters(error_kind: CloudErrorKind) -> CF_OPERATION_PARAMETERS_0 {
        CF_OPERATION_PARAMETERS_0 {
            AckDehydrate: CF_OPERATION_PARAMETERS_0_5 {
                Flags: CloudFilters::CF_OPERATION_ACK_DEHYDRATE_FLAG_NONE,
                CompletionStatus: error_kind.into(),
                FileIdentity: ptr::null(),
                FileIdentityLength: 0,
            },
        }
    }
}

/// Confirm deletion of the placeholder.
#[derive(Debug)]
pub struct Delete;

impl Command for Delete {
    const OPERATION: CF_OPERATION_TYPE = CloudFilters::CF_OPERATION_TYPE_ACK_DELETE;

    type Result = ();
    type Field = CF_OPERATION_PARAMETERS_0_7;

    unsafe fn result(_info: CF_OPERATION_PARAMETERS_0) -> Self::Result {}

    fn build(&self) -> CF_OPERATION_PARAMETERS_0 {
        CF_OPERATION_PARAMETERS_0 {
            AckDelete: CF_OPERATION_PARAMETERS_0_7 {
                Flags: CloudFilters::CF_OPERATION_ACK_DELETE_FLAG_NONE,
                CompletionStatus: Foundation::STATUS_SUCCESS,
            },
        }
    }
}

impl Fallible for Delete {
    fn fail(
        connection_key: RawConnectionKey,
        transfer_key: RawTransferKey,
        error_kind: CloudErrorKind,
    ) -> core::Result<Self::Result> {
        Self::fail_with_context(
            &OperationContext::from_keys(connection_key, transfer_key),
            error_kind,
        )
    }
}

impl Delete {
    /// Report callback failure using the original owned operation context.
    pub fn fail_with_context(
        context: &OperationContext,
        error_kind: CloudErrorKind,
    ) -> core::Result<<Self as Command>::Result> {
        execute_with_context::<Self>(Self::failure_parameters(error_kind), context)
    }

    fn failure_parameters(error_kind: CloudErrorKind) -> CF_OPERATION_PARAMETERS_0 {
        CF_OPERATION_PARAMETERS_0 {
            AckDelete: CF_OPERATION_PARAMETERS_0_7 {
                Flags: CloudFilters::CF_OPERATION_ACK_DELETE_FLAG_NONE,
                CompletionStatus: error_kind.into(),
            },
        }
    }
}

/// Confirm rename/move of the placeholder.
#[derive(Debug)]
pub struct Rename;

impl Command for Rename {
    const OPERATION: CF_OPERATION_TYPE = CloudFilters::CF_OPERATION_TYPE_ACK_RENAME;

    type Result = ();
    type Field = CF_OPERATION_PARAMETERS_0_6;

    unsafe fn result(_info: CF_OPERATION_PARAMETERS_0) -> Self::Result {}

    fn build(&self) -> CF_OPERATION_PARAMETERS_0 {
        CF_OPERATION_PARAMETERS_0 {
            AckRename: CF_OPERATION_PARAMETERS_0_6 {
                Flags: CloudFilters::CF_OPERATION_ACK_RENAME_FLAG_NONE,
                CompletionStatus: Foundation::STATUS_SUCCESS,
            },
        }
    }
}

impl Fallible for Rename {
    fn fail(
        connection_key: RawConnectionKey,
        transfer_key: RawTransferKey,
        error_kind: CloudErrorKind,
    ) -> core::Result<Self::Result> {
        Self::fail_with_context(
            &OperationContext::from_keys(connection_key, transfer_key),
            error_kind,
        )
    }
}

impl Rename {
    /// Report callback failure using the original owned operation context.
    pub fn fail_with_context(
        context: &OperationContext,
        error_kind: CloudErrorKind,
    ) -> core::Result<<Self as Command>::Result> {
        execute_with_context::<Self>(Self::failure_parameters(error_kind), context)
    }

    fn failure_parameters(error_kind: CloudErrorKind) -> CF_OPERATION_PARAMETERS_0 {
        CF_OPERATION_PARAMETERS_0 {
            AckRename: CF_OPERATION_PARAMETERS_0_6 {
                Flags: CloudFilters::CF_OPERATION_ACK_RENAME_FLAG_NONE,
                CompletionStatus: error_kind.into(),
            },
        }
    }
}

/// Transfer one directory page; only the last page disables on-demand population.
pub(crate) fn placeholder_batch(
    placeholders: &mut [PlaceholderFile],
    total: u64,
    final_batch: bool,
    context: &OperationContext,
) -> core::Result<u32> {
    placeholder_batch_with(
        placeholders,
        total,
        final_batch,
        context,
        super::executor::execute_raw_with_context::<CreatePlaceholders>,
    )
}

fn placeholder_batch_with(
    placeholders: &mut [PlaceholderFile],
    total: u64,
    final_batch: bool,
    context: &OperationContext,
    execute: impl FnOnce(
        CF_OPERATION_PARAMETERS_0,
        &OperationContext,
    ) -> core::Result<CF_OPERATION_PARAMETERS_0>,
) -> core::Result<u32> {
    let invalid = || core::Error::from(Foundation::E_INVALIDARG);
    let count = u32::try_from(placeholders.len()).map_err(|_| invalid())?;
    let total = i64::try_from(total).map_err(|_| invalid())?;
    if total < i64::from(count) {
        return Err(invalid());
    }
    let info = CF_OPERATION_PARAMETERS_0 {
        TransferPlaceholders: CF_OPERATION_PARAMETERS_0_4 {
            Flags: if final_batch {
                CloudFilters::CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAG_DISABLE_ON_DEMAND_POPULATION
            } else {
                CloudFilters::CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAG_NONE
            },
            CompletionStatus: Foundation::STATUS_SUCCESS,
            PlaceholderTotalCount: total,
            PlaceholderArray: if placeholders.is_empty() {
                ptr::null_mut()
            } else {
                placeholders.as_mut_ptr().cast()
            },
            PlaceholderCount: count,
            EntriesProcessed: 0,
        },
    };
    let output = execute(info, context)?;
    Ok(unsafe { output.TransferPlaceholders.EntriesProcessed })
}

#[cfg(test)]
mod context_tests {
    use super::*;
    use windows::Win32::Storage::CloudFilters::{CF_CALLBACK_INFO, CF_CONNECTION_KEY};

    fn assert_failure_context<C: Command>(parameters: CF_OPERATION_PARAMETERS_0) {
        for request_key in [0, 17] {
            let context = OperationContext::from_callback(&CF_CALLBACK_INFO {
                ConnectionKey: CF_CONNECTION_KEY(7),
                TransferKey: 11,
                RequestKey: request_key,
                ..Default::default()
            });
            super::super::executor::execute_with::<C>(parameters, &context, |operation, _| {
                assert_eq!(operation.Type, C::OPERATION);
                assert_eq!(operation.ConnectionKey.0, 7);
                assert_eq!(operation.TransferKey, 11);
                assert_eq!(operation.RequestKey, request_key);
                Ok(())
            })
            .unwrap();
            let refused =
                super::super::executor::execute_with::<C>(parameters, &context, |operation, _| {
                    assert_eq!(operation.RequestKey, request_key);
                    Err(Foundation::E_FAIL.into())
                });
            assert_eq!(refused.err().unwrap().code(), Foundation::E_FAIL);
        }
    }

    #[test]
    fn all_callback_failure_builders_preserve_status_range_and_context() {
        let error = CloudErrorKind::AuthenticationFailed;
        let expected: Foundation::NTSTATUS = error.into();
        let write = Write::failure_parameters(error, 4096, -1);
        assert_eq!(unsafe { write.TransferData.CompletionStatus }, expected);
        assert_eq!(unsafe { write.TransferData.Offset }, 4096);
        assert_eq!(unsafe { write.TransferData.Length }, -1);
        assert!(unsafe { write.TransferData.Buffer }.is_null());
        assert_failure_context::<Write>(write);
        let populate = CreatePlaceholders::failure_parameters(error);
        assert_eq!(
            unsafe { populate.TransferPlaceholders.CompletionStatus },
            expected
        );
        assert_failure_context::<CreatePlaceholders>(populate);
        let validate = Validate::failure_parameters(error);
        assert_eq!(unsafe { validate.AckData.CompletionStatus }, expected);
        assert_failure_context::<Validate>(validate);
        let dehydrate = Dehydrate::failure_parameters(error);
        assert_eq!(unsafe { dehydrate.AckDehydrate.CompletionStatus }, expected);
        assert_failure_context::<Dehydrate>(dehydrate);
        let delete = Delete::failure_parameters(error);
        assert_eq!(unsafe { delete.AckDelete.CompletionStatus }, expected);
        assert_failure_context::<Delete>(delete);
        let rename = Rename::failure_parameters(error);
        assert_eq!(unsafe { rename.AckRename.CompletionStatus }, expected);
        assert_failure_context::<Rename>(rename);
    }

    #[test]
    fn placeholder_batch_preserves_context_and_native_processed_count() {
        let context = OperationContext::from_callback(&CF_CALLBACK_INFO {
            ConnectionKey: CF_CONNECTION_KEY(7),
            TransferKey: 11,
            RequestKey: 17,
            ..Default::default()
        });
        let mut placeholders = [PlaceholderFile::new("a"), PlaceholderFile::new("b")];
        let processed =
            placeholder_batch_with(&mut placeholders, 2, true, &context, |info, context| {
                super::super::executor::execute_with::<CreatePlaceholders>(
                    info,
                    context,
                    |operation, parameters| {
                        assert_eq!(operation.ConnectionKey.0, 7);
                        assert_eq!(operation.TransferKey, 11);
                        assert_eq!(operation.RequestKey, 17);
                        assert_eq!(
                            unsafe { parameters.Anonymous.TransferPlaceholders.PlaceholderCount },
                            2
                        );
                        parameters.Anonymous.TransferPlaceholders.EntriesProcessed = 1;
                        Ok(())
                    },
                )
            })
            .unwrap();
        assert_eq!(processed, 1);
        for total in [1, u64::MAX] {
            assert!(
                placeholder_batch_with(&mut placeholders, total, false, &context, |_, _| {
                    panic!("Invalid batch reached the native executor")
                })
                .is_err()
            );
        }
        let refused = placeholder_batch_with(&mut placeholders, 2, true, &context, |_, _| {
            Err(Foundation::E_FAIL.into())
        });
        assert_eq!(refused.err().unwrap().code(), Foundation::E_FAIL);
    }
}

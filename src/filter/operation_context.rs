use std::{fmt, mem, ptr};

use windows::Win32::{
    Storage::CloudFilters::{
        CF_CALLBACK_INFO, CF_CONNECTION_KEY, CF_OPERATION_INFO, CF_OPERATION_TYPE,
        CF_REQUEST_KEY_DEFAULT,
    },
    System::CorrelationVector::CORRELATION_VECTOR,
};

use super::{RawConnectionKey, RawTransferKey};

/// Opaque callback values for matching an operation within its original lifecycle.
/// Keep the original callback and ticket owners when deciding whether work is live.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct OperationIdentity {
    connection: RawConnectionKey,
    transfer: RawTransferKey,
    request: i64,
    volume: u32,
    root: i64,
    file: i64,
}

impl fmt::Debug for OperationIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationIdentity")
            .finish_non_exhaustive()
    }
}

/// Owned native operation values copied while the callback metadata is valid.
#[derive(Clone)]
pub struct OperationContext {
    identity: OperationIdentity,
    correlation: Option<CORRELATION_VECTOR>,
}

impl OperationContext {
    pub(crate) fn from_callback(info: &CF_CALLBACK_INFO) -> Self {
        Self {
            identity: OperationIdentity {
                connection: info.ConnectionKey.0,
                transfer: info.TransferKey,
                request: info.RequestKey,
                volume: info.VolumeSerialNumber,
                root: info.SyncRootFileId,
                file: info.FileId,
            },
            // CFAPI owns this optional fixed-size value for the callback. Copy it
            // now so tickets and async consumers retain no borrowed pointer.
            correlation: unsafe { info.CorrelationVector.as_ref().copied() },
        }
    }

    pub(crate) fn from_keys(connection: RawConnectionKey, transfer: RawTransferKey) -> Self {
        Self {
            identity: OperationIdentity {
                connection,
                transfer,
                request: CF_REQUEST_KEY_DEFAULT as i64,
                volume: 0,
                root: 0,
                file: 0,
            },
            correlation: None,
        }
    }

    /// Preserve zero values and exact equality when comparing callback metadata.
    pub fn identity(&self) -> OperationIdentity {
        self.identity
    }

    pub(crate) fn connection_key(&self) -> RawConnectionKey {
        self.identity.connection
    }

    pub(crate) fn transfer_key(&self) -> RawTransferKey {
        self.identity.transfer
    }

    pub(crate) fn operation_info(&self, operation: CF_OPERATION_TYPE) -> CF_OPERATION_INFO {
        CF_OPERATION_INFO {
            StructSize: mem::size_of::<CF_OPERATION_INFO>() as u32,
            Type: operation,
            ConnectionKey: CF_CONNECTION_KEY(self.identity.connection),
            TransferKey: self.identity.transfer,
            CorrelationVector: self.correlation.as_ref().map_or(ptr::null(), |value| value),
            SyncStatus: ptr::null(),
            RequestKey: self.identity.request,
        }
    }
}

impl fmt::Debug for OperationContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationContext")
            .field("has_correlation", &self.correlation.is_some())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Storage::CloudFilters::CF_OPERATION_TYPE_TRANSFER_DATA;

    #[test]
    fn callback_context_owns_correlation_and_preserves_zero_and_reused_keys() {
        let mut correlation = CORRELATION_VECTOR::default();
        correlation.Version = 1;
        correlation.Vector[0] = 42;
        let mut info = CF_CALLBACK_INFO {
            ConnectionKey: CF_CONNECTION_KEY(7),
            TransferKey: 11,
            RequestKey: 0,
            FileId: 13,
            CorrelationVector: &correlation,
            ..Default::default()
        };
        let original = OperationContext::from_callback(&info);
        correlation.Vector[0] = 9;
        info.CorrelationVector = ptr::null();
        let repeated = OperationContext::from_callback(&info);
        assert_eq!(original.identity(), repeated.identity());
        let operation = original.operation_info(CF_OPERATION_TYPE_TRANSFER_DATA);
        assert_eq!(operation.RequestKey, 0);
        assert_eq!(unsafe { (*operation.CorrelationVector).Vector[0] }, 42);
        assert!(repeated
            .operation_info(CF_OPERATION_TYPE_TRANSFER_DATA)
            .CorrelationVector
            .is_null());
        for changed in [
            CF_CALLBACK_INFO {
                RequestKey: 17,
                ..info
            },
            CF_CALLBACK_INFO {
                TransferKey: 19,
                ..info
            },
            CF_CALLBACK_INFO {
                ConnectionKey: CF_CONNECTION_KEY(23),
                ..info
            },
            CF_CALLBACK_INFO { FileId: 29, ..info },
        ] {
            assert_ne!(
                original.identity(),
                OperationContext::from_callback(&changed).identity()
            );
        }
    }
}

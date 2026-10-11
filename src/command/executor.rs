use std::{
    mem::{self, offset_of},
    ptr,
};

use windows::{
    core,
    Win32::Storage::CloudFilters::{
        self, CfExecute, CF_CONNECTION_KEY, CF_OPERATION_INFO, CF_OPERATION_PARAMETERS,
        CF_OPERATION_PARAMETERS_0, CF_OPERATION_TYPE,
    },
};

use crate::{
    error::CloudErrorKind,
    filter::{RawConnectionKey, RawTransferKey},
};

/// Allows a command to fail with a specified error.
pub trait Fallible: Command {
    fn fail(
        connection_key: RawConnectionKey,
        transfer_key: RawTransferKey,
        error_kind: CloudErrorKind,
    ) -> core::Result<Self::Result>;
}

/// A Cloud Filter command used to execute various functions.
pub trait Command: Sized {
    /// The [CF_OPERATION_TYPE][windows::Win32::Storage::CloudFilters::CF_OPERATION_TYPE]
    /// corresponding to the command.
    const OPERATION: CF_OPERATION_TYPE;
    /// The result of the command, if applicable.
    type Result;
    /// The union field of
    /// [CF_OPERATION_PARAMETERS_0][windows::Win32::Storage::CloudFilters::CF_OPERATION_PARAMETERS_0].
    type Field;

    /// Gathers and returns the result of the command.
    ///
    /// # Safety
    /// It is expected to index the union parameter, hence `unsafe`.
    unsafe fn result(info: CF_OPERATION_PARAMETERS_0) -> Self::Result;

    /// Creates and returns the union.
    fn build(&self) -> CF_OPERATION_PARAMETERS_0;

    /// Executes the command to the platform.
    fn execute(
        &self,
        connection_key: RawConnectionKey,
        transfer_key: RawTransferKey,
    ) -> core::Result<Self::Result> {
        execute::<Self>(self.build(), connection_key, transfer_key)
    }
}

pub fn execute<C: Command>(
    info: CF_OPERATION_PARAMETERS_0,
    connection_key: RawConnectionKey,
    transfer_key: RawTransferKey,
) -> core::Result<C::Result> {
    let output = execute_raw::<C>(info, connection_key, transfer_key)?;
    Ok(unsafe { C::result(output) })
}

pub(crate) fn execute_raw<C: Command>(
    info: CF_OPERATION_PARAMETERS_0,
    connection_key: RawConnectionKey,
    transfer_key: RawTransferKey,
) -> core::Result<CF_OPERATION_PARAMETERS_0> {
    execute_with::<C>(
        info,
        connection_key,
        transfer_key,
        |operation, parameters| unsafe { CfExecute(operation, parameters) },
    )
}

fn execute_with<C: Command>(
    info: CF_OPERATION_PARAMETERS_0,
    connection_key: RawConnectionKey,
    transfer_key: RawTransferKey,
    invoke: impl FnOnce(&CF_OPERATION_INFO, &mut CF_OPERATION_PARAMETERS) -> core::Result<()>,
) -> core::Result<CF_OPERATION_PARAMETERS_0> {
    let operation = CF_OPERATION_INFO {
        StructSize: mem::size_of::<CF_OPERATION_INFO>() as u32,
        Type: C::OPERATION,
        ConnectionKey: CF_CONNECTION_KEY(connection_key),
        TransferKey: transfer_key,
        CorrelationVector: ptr::null(),
        SyncStatus: ptr::null(),
        // Request context propagation is tracked separately in fork #5.
        RequestKey: CloudFilters::CF_REQUEST_KEY_DEFAULT as i64,
    };
    let mut parameters = CF_OPERATION_PARAMETERS {
        ParamSize: (mem::size_of::<C::Field>() + offset_of!(CF_OPERATION_PARAMETERS, Anonymous))
            as u32,
        Anonymous: info,
    };
    invoke(&operation, &mut parameters)?;
    Ok(parameters.Anonymous)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Read;
    use windows::Win32::Foundation::E_FAIL;

    #[test]
    fn returns_native_output_and_preserves_failure() {
        let mut buffer = [0u8; 42];
        let command = Read {
            buffer: &mut buffer,
            position: 0,
        };
        let output = execute_with::<Read>(command.build(), 1, 2, |_, params| {
            params.Anonymous.RetrieveData.ReturnedLength = 42;
            Ok(())
        })
        .unwrap();
        assert_eq!(unsafe { Read::result(output) }, 42);
        let failed = execute_with::<Read>(command.build(), 1, 2, |_, _| Err(E_FAIL.into()));
        assert_eq!(failed.err().unwrap().code(), E_FAIL);
    }
}

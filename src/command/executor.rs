use std::mem::{self, offset_of};

use windows::{
    core,
    Win32::Storage::CloudFilters::{
        CfExecute, CF_OPERATION_INFO, CF_OPERATION_PARAMETERS, CF_OPERATION_PARAMETERS_0,
        CF_OPERATION_TYPE,
    },
};

use crate::{
    error::CloudErrorKind,
    filter::{OperationContext, RawConnectionKey, RawTransferKey},
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

    /// Execute using the owned context from the original callback or ticket.
    fn execute_with_context(&self, context: &OperationContext) -> core::Result<Self::Result> {
        execute_with_context::<Self>(self.build(), context)
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
    execute_raw_with_context::<C>(
        info,
        &OperationContext::from_keys(connection_key, transfer_key),
    )
}

pub(crate) fn execute_with_context<C: Command>(
    info: CF_OPERATION_PARAMETERS_0,
    context: &OperationContext,
) -> core::Result<C::Result> {
    let output = execute_raw_with_context::<C>(info, context)?;
    Ok(unsafe { C::result(output) })
}

pub(crate) fn execute_raw_with_context<C: Command>(
    info: CF_OPERATION_PARAMETERS_0,
    context: &OperationContext,
) -> core::Result<CF_OPERATION_PARAMETERS_0> {
    execute_with::<C>(info, context, |operation, parameters| unsafe {
        CfExecute(operation, parameters)
    })
}

pub(super) fn execute_with<C: Command>(
    info: CF_OPERATION_PARAMETERS_0,
    context: &OperationContext,
    invoke: impl FnOnce(&CF_OPERATION_INFO, &mut CF_OPERATION_PARAMETERS) -> core::Result<()>,
) -> core::Result<CF_OPERATION_PARAMETERS_0> {
    let operation = context.operation_info(C::OPERATION);
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
        let context = OperationContext::from_keys(1, 2);
        let output = execute_with::<Read>(command.build(), &context, |operation, params| {
            assert_eq!(operation.ConnectionKey.0, 1);
            assert_eq!(operation.TransferKey, 2);
            assert_eq!(
                operation.RequestKey,
                windows::Win32::Storage::CloudFilters::CF_REQUEST_KEY_DEFAULT as i64
            );
            assert!(operation.CorrelationVector.is_null());
            params.Anonymous.RetrieveData.ReturnedLength = 42;
            Ok(())
        })
        .unwrap();
        assert_eq!(unsafe { Read::result(output) }, 42);
        let failed = execute_with::<Read>(command.build(), &context, |_, _| Err(E_FAIL.into()));
        assert_eq!(failed.err().unwrap().code(), E_FAIL);
    }

    #[test]
    fn callback_request_and_owned_correlation_reach_the_actual_executor() {
        use windows::Win32::{
            Storage::CloudFilters::{CF_CALLBACK_INFO, CF_CONNECTION_KEY},
            System::CorrelationVector::CORRELATION_VECTOR,
        };
        let mut correlation = CORRELATION_VECTOR::default();
        correlation.Vector[0] = 42;
        let context = OperationContext::from_callback(&CF_CALLBACK_INFO {
            ConnectionKey: CF_CONNECTION_KEY(7),
            TransferKey: 11,
            RequestKey: 17,
            CorrelationVector: &correlation,
            ..Default::default()
        });
        correlation.Vector[0] = 9;
        let mut buffer = [0u8; 42];
        let command = Read {
            buffer: &mut buffer,
            position: 0,
        };
        let returned = execute_with::<Read>(command.build(), &context, |operation, parameters| {
            assert_eq!(operation.ConnectionKey.0, 7);
            assert_eq!(operation.TransferKey, 11);
            assert_eq!(operation.RequestKey, 17);
            assert_eq!(unsafe { (*operation.CorrelationVector).Vector[0] }, 42);
            parameters.Anonymous.RetrieveData.ReturnedLength = 23;
            Ok(())
        })
        .unwrap();
        assert_eq!(unsafe { Read::result(returned) }, 23);
        assert_eq!(
            execute_with::<Read>(command.build(), &context, |operation, _| {
                assert_eq!(operation.RequestKey, 17);
                Err(E_FAIL.into())
            })
            .err()
            .unwrap()
            .code(),
            E_FAIL
        );
    }
}

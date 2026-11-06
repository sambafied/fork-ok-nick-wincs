use core::{
    mem::{offset_of, size_of},
    ptr,
};

use windows::Win32::Storage::CloudFilters::{CF_CALLBACK_INFO, CF_CALLBACK_PARAMETERS};

use super::metadata::MetadataError;

/// Reads only the callback's SDK size header.
///
/// # Safety
/// A non-null `info` must expose a readable SDK `StructSize` header.
/// This checks the advertised extent, not arbitrary pointer validity.
pub(super) unsafe fn callback_size(info: *const CF_CALLBACK_INFO) -> Result<usize, MetadataError> {
    if info.is_null() {
        return Err(MetadataError::TruncatedCallback);
    }

    let announced = unsafe { ptr::read_unaligned(info.cast::<u32>()) } as usize;
    if announced < size_of::<u32>() {
        return Err(MetadataError::TruncatedCallback);
    }
    Ok(announced)
}

/// Reads a callback field only after its complete end has been admitted.
///
/// # Safety
/// The SDK buffer must be readable through its advertised extent. `field_end`
/// must be the complete byte end of the field returned by `field`, relative to
/// `info`. The closure must return that field's address without dereferencing
/// other fields. Its stored representation must be valid for `T`.
pub(super) unsafe fn callback_field<T: Copy>(
    info: *const CF_CALLBACK_INFO,
    field_end: usize,
    field: impl FnOnce(*const CF_CALLBACK_INFO) -> *const T,
) -> Result<T, MetadataError> {
    let announced = unsafe { callback_size(info)? };
    if field_end < size_of::<T>() || announced < field_end {
        return Err(MetadataError::TruncatedCallback);
    }

    // Do not even evaluate the field-address closure before admission.
    let address = field(info);
    Ok(unsafe { ptr::read_unaligned(address) })
}

/// Copies the known callback ABI prefix, admitting future larger structures.
///
/// # Safety
/// The non-null SDK buffer must expose a readable size header and be readable
/// through its advertised extent, with a valid `CF_CALLBACK_INFO` prefix.
pub(super) unsafe fn copy_callback_info(
    info: *const CF_CALLBACK_INFO,
) -> Result<CF_CALLBACK_INFO, MetadataError> {
    if unsafe { callback_size(info)? } < size_of::<CF_CALLBACK_INFO>() {
        return Err(MetadataError::TruncatedCallback);
    }

    Ok(unsafe { ptr::read_unaligned(info) })
}

/// Copies only the requested, admitted prefix of a callback parameter arm.
///
/// # Safety
/// A non-null `params` must expose a readable SDK `ParamSize` header and be
/// readable through the advertised extent. `T` must be the corresponding CF
/// ABI arm with a valid all-zero default. `required_bytes` must cover the
/// complete ends of every consumed field, and the copied native bytes combined
/// with the default remainder must be a valid `T`.
///
/// This helper does not validate pointers, discriminants, flags, or ranges.
pub(super) unsafe fn read_parameter_arm<T: Default + Copy>(
    params: *const CF_CALLBACK_PARAMETERS,
    required_bytes: usize,
) -> Result<T, MetadataError> {
    if params.is_null() {
        return Err(MetadataError::TruncatedParameters);
    }

    let announced = unsafe { ptr::read_unaligned(params.cast::<u32>()) } as usize;
    let union_offset = offset_of!(CF_CALLBACK_PARAMETERS, Anonymous);
    let required_end = union_offset
        .checked_add(required_bytes)
        .ok_or(MetadataError::TruncatedParameters)?;

    if required_bytes == 0 || required_bytes > size_of::<T>() || announced < required_end {
        return Err(MetadataError::TruncatedParameters);
    }

    let mut result = T::default();
    // Form the union address only after the advertised extent is admitted.
    // Copy the requested prefix, never the arm's remaining fields or tail.
    unsafe {
        ptr::copy_nonoverlapping(
            params.cast::<u8>().add(union_offset),
            ptr::addr_of_mut!(result).cast::<u8>(),
            required_bytes,
        );
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::{cell::Cell, mem::align_of};
    use windows::Win32::Storage::CloudFilters::{
        CF_CALLBACK_CANCEL_FLAGS, CF_CALLBACK_FETCH_DATA_FLAGS, CF_CALLBACK_PARAMETERS_0,
        CF_CALLBACK_PARAMETERS_0_0, CF_CALLBACK_PARAMETERS_0_1,
    };

    fn header_only() -> Box<u32> {
        Box::new(size_of::<u32>() as u32)
    }

    #[test]
    fn null_inputs_are_refused() {
        unsafe {
            assert!(matches!(
                callback_size(ptr::null()),
                Err(MetadataError::TruncatedCallback)
            ));
            assert!(matches!(
                copy_callback_info(ptr::null()),
                Err(MetadataError::TruncatedCallback)
            ));
            assert!(matches!(
                callback_field::<u32>(ptr::null(), size_of::<u32>(), |_| {
                    panic!("null callback must not evaluate the field closure")
                }),
                Err(MetadataError::TruncatedCallback)
            ));
            assert!(matches!(
                read_parameter_arm::<CF_CALLBACK_PARAMETERS_0_0>(
                    ptr::null(),
                    size_of::<CF_CALLBACK_CANCEL_FLAGS>(),
                ),
                Err(MetadataError::TruncatedParameters)
            ));
        }
    }

    #[test]
    fn actual_four_byte_callback_allocation_refuses_full_copy_and_field() {
        let header = header_only();
        let info = ptr::from_ref(header.as_ref()).cast::<CF_CALLBACK_INFO>();
        let calls = Cell::new(0);

        unsafe {
            assert_eq!(callback_size(info).unwrap(), size_of::<u32>());
            assert!(matches!(
                copy_callback_info(info),
                Err(MetadataError::TruncatedCallback)
            ));

            let result = callback_field::<i64>(
                info,
                offset_of!(CF_CALLBACK_INFO, RequestKey) + size_of::<i64>(),
                |_| {
                    calls.set(calls.get() + 1);
                    // Deliberately unreadable: admission must prevent invocation.
                    ptr::null()
                },
            );
            assert!(matches!(result, Err(MetadataError::TruncatedCallback)));
        }

        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn actual_four_byte_parameter_allocation_refuses_union_read() {
        let header = header_only();
        let params = ptr::from_ref(header.as_ref()).cast::<CF_CALLBACK_PARAMETERS>();

        assert!(matches!(
            unsafe {
                read_parameter_arm::<CF_CALLBACK_PARAMETERS_0_0>(
                    params,
                    size_of::<CF_CALLBACK_CANCEL_FLAGS>(),
                )
            },
            Err(MetadataError::TruncatedParameters)
        ));
    }

    #[test]
    fn callback_field_rejects_end_smaller_than_field_width() {
        let info = CF_CALLBACK_INFO {
            StructSize: size_of::<CF_CALLBACK_INFO>() as u32,
            ..Default::default()
        };
        let calls = Cell::new(0);

        let result = unsafe {
            callback_field::<u32>(&info, size_of::<u32>() - 1, |_| {
                calls.set(calls.get() + 1);
                ptr::null()
            })
        };

        assert!(matches!(result, Err(MetadataError::TruncatedCallback)));
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn full_callback_and_future_larger_callback_are_admitted() {
        #[repr(C)]
        struct Extended {
            info: CF_CALLBACK_INFO,
            future: [u8; 16],
        }

        let mut extended = Extended {
            info: CF_CALLBACK_INFO {
                StructSize: size_of::<CF_CALLBACK_INFO>() as u32,
                RequestKey: 73,
                FileSize: -1,
                ..Default::default()
            },
            future: [0xA5; 16],
        };

        unsafe {
            let full = copy_callback_info(&extended.info).unwrap();
            assert_eq!(full.RequestKey, 73);
            assert_eq!(full.FileSize, -1);

            extended.info.StructSize = size_of::<Extended>() as u32;
            let future = copy_callback_info(&extended.info).unwrap();
            assert_eq!(future.StructSize as usize, size_of::<Extended>());
            assert_eq!(future.RequestKey, 73);

            let request_key = callback_field::<i64>(
                &extended.info,
                offset_of!(CF_CALLBACK_INFO, RequestKey) + size_of::<i64>(),
                |info| ptr::addr_of!((*info).RequestKey),
            )
            .unwrap();
            assert_eq!(request_key, 73);
        }
    }

    #[test]
    fn flags_only_cancel_population_uses_real_union_offset() {
        let union_offset = offset_of!(CF_CALLBACK_PARAMETERS, Anonymous);
        let union_alignment = align_of::<CF_CALLBACK_PARAMETERS_0>();
        let expected_offset = size_of::<u32>().next_multiple_of(union_alignment);
        assert_eq!(union_offset, expected_offset);

        // The allocation ends immediately after Flags. There is no readable
        // cancel-fetch-data range or full parameter structure behind it.
        let flags_bytes = size_of::<CF_CALLBACK_CANCEL_FLAGS>();
        let mut buffer = vec![0u8; union_offset + flags_bytes];
        let announced = buffer.len() as u32;
        let native_flags = CF_CALLBACK_CANCEL_FLAGS(0x4000_0001);

        unsafe {
            ptr::write_unaligned(buffer.as_mut_ptr().cast::<u32>(), announced);
            ptr::write_unaligned(
                buffer
                    .as_mut_ptr()
                    .add(union_offset)
                    .cast::<CF_CALLBACK_CANCEL_FLAGS>(),
                native_flags,
            );

            let params = buffer.as_ptr().cast::<CF_CALLBACK_PARAMETERS>();
            let cancel =
                read_parameter_arm::<CF_CALLBACK_PARAMETERS_0_0>(params, flags_bytes).unwrap();
            assert_eq!(cancel.Flags.0, native_flags.0);
            assert_eq!(cancel.Anonymous.FetchData.FileOffset, 0);
            assert_eq!(cancel.Anonymous.FetchData.Length, 0);

            let range_end =
                offset_of!(CF_CALLBACK_PARAMETERS_0_0, Anonymous) + size_of::<i64>() * 2;
            assert!(matches!(
                read_parameter_arm::<CF_CALLBACK_PARAMETERS_0_0>(params, range_end),
                Err(MetadataError::TruncatedParameters)
            ));
            assert!(matches!(
                read_parameter_arm::<CF_CALLBACK_PARAMETERS_0_1>(
                    params,
                    offset_of!(CF_CALLBACK_PARAMETERS_0_1, RequiredLength) + size_of::<i64>(),
                ),
                Err(MetadataError::TruncatedParameters)
            ));
        }
    }

    #[test]
    fn invalid_requested_lengths_are_refused_before_copy() {
        let header = header_only();
        let params = ptr::from_ref(header.as_ref()).cast::<CF_CALLBACK_PARAMETERS>();

        for required_bytes in [0, size_of::<CF_CALLBACK_PARAMETERS_0_0>() + 1, usize::MAX] {
            assert!(matches!(
                unsafe { read_parameter_arm::<CF_CALLBACK_PARAMETERS_0_0>(params, required_bytes) },
                Err(MetadataError::TruncatedParameters)
            ));
        }
    }

    #[test]
    fn full_fetch_data_preserves_native_flags_and_eof_range_bits() {
        let source = CF_CALLBACK_PARAMETERS_0_1 {
            Flags: CF_CALLBACK_FETCH_DATA_FLAGS(0x4000_0001),
            RequiredFileOffset: 37,
            RequiredLength: -1,
            OptionalFileOffset: -1,
            OptionalLength: -1,
            LastDehydrationTime: -1,
            ..Default::default()
        };
        let mut params = CF_CALLBACK_PARAMETERS {
            ParamSize: size_of::<CF_CALLBACK_PARAMETERS>() as u32,
            ..Default::default()
        };
        params.Anonymous.FetchData = source;

        let copied = unsafe {
            read_parameter_arm::<CF_CALLBACK_PARAMETERS_0_1>(
                &params,
                size_of::<CF_CALLBACK_PARAMETERS_0_1>(),
            )
        }
        .unwrap();

        assert_eq!(copied, source);
        assert_eq!(copied.RequiredLength, -1);
        assert_eq!(copied.OptionalFileOffset, -1);
        assert_eq!(copied.OptionalLength, -1);
    }
}

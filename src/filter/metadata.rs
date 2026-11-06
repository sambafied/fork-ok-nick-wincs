use core::ffi::c_void;

use windows::core::PCWSTR;

/// Redacted validation categories. Contains no native metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataError {
    TruncatedCallback,
    TruncatedParameters,
    InvalidString,
    InvalidBlob,
    InvalidScalar,
    Allocation,
}

pub(crate) const MAX_WIDE_UNITS: usize = 32768;

/// Copy a bounded, NUL-terminated native string without retaining its pointer.
///
/// # Safety
///
/// A non-null pointer must identify a readable UTF-16 SDK buffer through its
/// terminator, or through MAX_WIDE_UNITS units if no terminator exists.
/// Address arithmetic checks do not validate arbitrary non-null addresses.
pub(crate) unsafe fn copy_wide(
    pointer: PCWSTR,
    required: bool,
) -> Result<Option<Vec<u16>>, MetadataError> {
    if pointer.0.is_null() {
        return if required {
            Err(MetadataError::InvalidString)
        } else {
            Ok(None)
        };
    }
    let mut length = None;
    for index in 0..MAX_WIDE_UNITS {
        let offset = index
            .checked_mul(std::mem::size_of::<u16>())
            .ok_or(MetadataError::InvalidString)?;
        (pointer.0 as usize)
            .checked_add(offset)
            .and_then(|address| address.checked_add(std::mem::size_of::<u16>()))
            .ok_or(MetadataError::InvalidString)?;
        if unsafe { pointer.0.add(index).read_unaligned() } == 0 {
            length = Some(index);
            break;
        }
    }
    let length = length.ok_or(MetadataError::InvalidString)?;
    let mut value = Vec::new();
    value
        .try_reserve_exact(length)
        .map_err(|_| MetadataError::Allocation)?;
    for index in 0..length {
        value.push(unsafe { pointer.0.add(index).read_unaligned() });
    }
    Ok(Some(value))
}

/// Copy a native identity blob with an explicit allocation limit.
///
/// # Safety
///
/// For nonzero length the pointer must identify a readable SDK buffer of at
/// least length bytes. This function cannot validate arbitrary addresses.
pub(crate) unsafe fn copy_blob(
    pointer: *const c_void,
    length: usize,
    maximum: usize,
) -> Result<Vec<u8>, MetadataError> {
    if length > maximum || length > isize::MAX as usize {
        return Err(MetadataError::InvalidBlob);
    }
    if length == 0 {
        return Ok(Vec::new());
    }
    if pointer.is_null() || (pointer as usize).checked_add(length).is_none() {
        return Err(MetadataError::InvalidBlob);
    }
    let mut value = Vec::new();
    value
        .try_reserve_exact(length)
        .map_err(|_| MetadataError::Allocation)?;
    // No native slice is formed for a null zero-length buffer.
    value.extend_from_slice(unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), length) });
    Ok(value)
}

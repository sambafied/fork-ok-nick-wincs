#![allow(clippy::missing_safety_doc)]

use windows::Win32::Storage::CloudFilters::{
    self, CF_CALLBACK_INFO, CF_CALLBACK_PARAMETERS, CF_CALLBACK_REGISTRATION,
};

use crate::{
    command::{self, Fallible},
    filter::{info, ticket, Request, SyncFilter},
};

pub type Callbacks = [CF_CALLBACK_REGISTRATION; 14];

macro_rules! callbacks {
    ($([$type:path, $name:ident]),*) => {
        [
            $(
                CF_CALLBACK_REGISTRATION {
                    Type: $type,
                    Callback: Some($name::<T>),
                },
            )*
            CF_CALLBACK_REGISTRATION {
                Type: CloudFilters::CF_CALLBACK_TYPE_NONE,
                Callback: None,
            },
        ]
    };
}

// TODO: const this
pub fn callbacks<T: SyncFilter + 'static>() -> Callbacks {
    callbacks!(
        [CloudFilters::CF_CALLBACK_TYPE_FETCH_DATA, fetch_data],
        [CloudFilters::CF_CALLBACK_TYPE_VALIDATE_DATA, validate_data],
        [
            CloudFilters::CF_CALLBACK_TYPE_CANCEL_FETCH_DATA,
            cancel_fetch_data
        ],
        [
            CloudFilters::CF_CALLBACK_TYPE_FETCH_PLACEHOLDERS,
            fetch_placeholders
        ],
        [
            CloudFilters::CF_CALLBACK_TYPE_CANCEL_FETCH_PLACEHOLDERS,
            cancel_fetch_placeholders
        ],
        [
            CloudFilters::CF_CALLBACK_TYPE_NOTIFY_FILE_OPEN_COMPLETION,
            notify_file_open_completion
        ],
        [
            CloudFilters::CF_CALLBACK_TYPE_NOTIFY_FILE_CLOSE_COMPLETION,
            notify_file_close_completion
        ],
        [
            CloudFilters::CF_CALLBACK_TYPE_NOTIFY_DEHYDRATE,
            notify_dehydrate
        ],
        [
            CloudFilters::CF_CALLBACK_TYPE_NOTIFY_DEHYDRATE_COMPLETION,
            notify_dehydrate_completion
        ],
        [CloudFilters::CF_CALLBACK_TYPE_NOTIFY_DELETE, notify_delete],
        [
            CloudFilters::CF_CALLBACK_TYPE_NOTIFY_DELETE_COMPLETION,
            notify_delete_completion
        ],
        [CloudFilters::CF_CALLBACK_TYPE_NOTIFY_RENAME, notify_rename],
        [
            CloudFilters::CF_CALLBACK_TYPE_NOTIFY_RENAME_COMPLETION,
            notify_rename_completion
        ]
    )
}

use super::{
    metadata::MetadataError,
    proxy_metadata::{callback_field, copy_callback_info, read_parameter_arm},
};
use crate::error::CloudErrorKind;
use core::{
    mem::{offset_of, size_of, size_of_val},
    ptr,
};

// The complete ends of consumed ABI fields, including real C-layout padding.
macro_rules! field_end {
    ($ty:ty, $field:ident) => {
        offset_of!($ty, $field) + size_of_val(&<$ty>::default().$field)
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailureKind {
    FetchData,
    ValidateData,
    FetchPlaceholders,
    Dehydrate,
    Delete,
    Rename,
    Notification,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FailureRoute {
    connection_key: i64,
    transfer_key: i64,
    range: Option<(i64, i64)>,
}

// Scheduling data only; extraction never invokes CFAPI or fabricates an owner.
unsafe fn failure_route(
    kind: FailureKind,
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) -> Option<FailureRoute> {
    if kind == FailureKind::Notification {
        return None;
    }
    let connection = unsafe {
        callback_field(info, field_end!(CF_CALLBACK_INFO, ConnectionKey), |p| {
            ptr::addr_of!((*p).ConnectionKey)
        })
    }
    .ok()?;
    let transfer_key = unsafe {
        callback_field(info, field_end!(CF_CALLBACK_INFO, TransferKey), |p| {
            ptr::addr_of!((*p).TransferKey)
        })
    }
    .ok()?;
    let range = if kind == FailureKind::FetchData {
        let raw = unsafe {
            read_parameter_arm::<CloudFilters::CF_CALLBACK_PARAMETERS_0_1>(
                params,
                field_end!(CloudFilters::CF_CALLBACK_PARAMETERS_0_1, RequiredLength),
            )
        }
        .ok()?;
        // Preserve SDK range values, including CF_EOF == -1; no invented zero.
        Some((raw.RequiredFileOffset, raw.RequiredLength))
    } else {
        None
    };
    Some(FailureRoute {
        connection_key: connection.0,
        transfer_key,
        range,
    })
}

fn report_failure(kind: FailureKind, route: FailureRoute, error: CloudErrorKind) {
    let result = match kind {
        FailureKind::FetchData => match route.range {
            Some((offset, length)) => command::Write::fail_range(
                route.connection_key,
                route.transfer_key,
                error,
                offset,
                length,
            ),
            None => return,
        },
        FailureKind::ValidateData => {
            command::Validate::fail(route.connection_key, route.transfer_key, error)
        }
        FailureKind::FetchPlaceholders => {
            command::CreatePlaceholders::fail(route.connection_key, route.transfer_key, error)
        }
        FailureKind::Dehydrate => {
            command::Dehydrate::fail(route.connection_key, route.transfer_key, error)
        }
        FailureKind::Delete => {
            command::Delete::fail(route.connection_key, route.transfer_key, error)
        }
        FailureKind::Rename => {
            command::Rename::fail(route.connection_key, route.transfer_key, error)
        }
        FailureKind::Notification => return,
    };
    if result.is_err() {
        log::error!("Cannot report callback failure");
    }
}

unsafe fn reject_callback<T: SyncFilter>(
    filter: &T,
    error: MetadataError,
    kind: FailureKind,
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    filter.metadata_error(error);
    if let Some(route) = unsafe { failure_route(kind, info, params) } {
        report_failure(kind, route, CloudErrorKind::InvalidRequest);
    }
}

unsafe fn capture_request(info: *const CF_CALLBACK_INFO) -> Result<Request, MetadataError> {
    unsafe { Request::new(copy_callback_info(info)?) }
}

pub unsafe extern "system" fn fetch_data<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    let Some(filter) = (unsafe { filter_from_info::<T>(info) }) else {
        return;
    };
    let captured = (|| -> Result<_, MetadataError> {
        let request = unsafe { capture_request(info)? };
        let raw = unsafe {
            read_parameter_arm::<CloudFilters::CF_CALLBACK_PARAMETERS_0_1>(
                params,
                field_end!(
                    CloudFilters::CF_CALLBACK_PARAMETERS_0_1,
                    LastDehydrationReason
                ),
            )?
        };
        let details = info::FetchData(raw);
        Ok((request, details))
    })();
    let (request, details) = match captured {
        Ok(value) => value,
        Err(error) => {
            unsafe { reject_callback(&*filter, error, FailureKind::FetchData, info, params) };
            return;
        }
    };
    let route = unsafe { failure_route(FailureKind::FetchData, info, params) };
    let ticket = ticket::FetchData::new(request.connection_key(), request.transfer_key());
    if let Err(error) = filter.fetch_data(request, ticket, details) {
        if let Some(route) = route {
            report_failure(FailureKind::FetchData, route, error);
        }
    }
}

pub unsafe extern "system" fn validate_data<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    let Some(filter) = (unsafe { filter_from_info::<T>(info) }) else {
        return;
    };
    let captured = (|| -> Result<_, MetadataError> {
        let request = unsafe { capture_request(info)? };
        let raw = unsafe {
            read_parameter_arm::<CloudFilters::CF_CALLBACK_PARAMETERS_0_2>(
                params,
                field_end!(CloudFilters::CF_CALLBACK_PARAMETERS_0_2, RequiredLength),
            )?
        };
        let details = info::ValidateData(raw);
        Ok((request, details))
    })();
    let (request, details) = match captured {
        Ok(value) => value,
        Err(error) => {
            unsafe { reject_callback(&*filter, error, FailureKind::ValidateData, info, params) };
            return;
        }
    };
    let route = unsafe { failure_route(FailureKind::ValidateData, info, params) };
    let ticket = ticket::ValidateData::new(request.connection_key(), request.transfer_key());
    if let Err(error) = filter.validate_data(request, ticket, details) {
        if let Some(route) = route {
            report_failure(FailureKind::ValidateData, route, error);
        }
    }
}

pub unsafe extern "system" fn cancel_fetch_data<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    let Some(filter) = (unsafe { filter_from_info::<T>(info) }) else {
        return;
    };
    let captured = (|| -> Result<_, MetadataError> {
        let request = unsafe { capture_request(info)? };
        let raw = unsafe {
            read_parameter_arm::<CloudFilters::CF_CALLBACK_PARAMETERS_0_0>(
                params,
                field_end!(CloudFilters::CF_CALLBACK_PARAMETERS_0_0, Anonymous),
            )?
        };
        let details = info::CancelFetchData(raw);
        Ok((request, details))
    })();
    let (request, details) = match captured {
        Ok(value) => value,
        Err(error) => {
            unsafe { reject_callback(&*filter, error, FailureKind::Notification, info, params) };
            return;
        }
    };
    filter.cancel_fetch_data(request, details);
}

pub unsafe extern "system" fn fetch_placeholders<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    let Some(filter) = (unsafe { filter_from_info::<T>(info) }) else {
        return;
    };
    let captured = (|| -> Result<_, MetadataError> {
        let request = unsafe { capture_request(info)? };
        let raw = unsafe {
            read_parameter_arm::<CloudFilters::CF_CALLBACK_PARAMETERS_0_3>(
                params,
                field_end!(CloudFilters::CF_CALLBACK_PARAMETERS_0_3, Pattern),
            )?
        };
        let details = unsafe { info::FetchPlaceholders::new(raw)? };
        Ok((request, details))
    })();
    let (request, details) = match captured {
        Ok(value) => value,
        Err(error) => {
            unsafe {
                reject_callback(
                    &*filter,
                    error,
                    FailureKind::FetchPlaceholders,
                    info,
                    params,
                )
            };
            return;
        }
    };
    let route = unsafe { failure_route(FailureKind::FetchPlaceholders, info, params) };
    let ticket = ticket::FetchPlaceholders::new(request.connection_key(), request.transfer_key());
    if let Err(error) = filter.fetch_placeholders(request, ticket, details) {
        if let Some(route) = route {
            report_failure(FailureKind::FetchPlaceholders, route, error);
        }
    }
}

pub unsafe extern "system" fn cancel_fetch_placeholders<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    let Some(filter) = (unsafe { filter_from_info::<T>(info) }) else {
        return;
    };
    let captured = (|| -> Result<_, MetadataError> {
        let request = unsafe { capture_request(info)? };
        let raw = unsafe {
            read_parameter_arm::<CloudFilters::CF_CALLBACK_PARAMETERS_0_0>(
                params,
                field_end!(CloudFilters::CF_CALLBACK_PARAMETERS_0_0, Flags),
            )?
        };
        let details = info::CancelFetchPlaceholders(raw);
        Ok((request, details))
    })();
    let (request, details) = match captured {
        Ok(value) => value,
        Err(error) => {
            unsafe { reject_callback(&*filter, error, FailureKind::Notification, info, params) };
            return;
        }
    };
    filter.cancel_fetch_placeholders(request, details);
}

pub unsafe extern "system" fn notify_file_open_completion<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    let Some(filter) = (unsafe { filter_from_info::<T>(info) }) else {
        return;
    };
    let captured = (|| -> Result<_, MetadataError> {
        let request = unsafe { capture_request(info)? };
        let raw = unsafe {
            read_parameter_arm::<CloudFilters::CF_CALLBACK_PARAMETERS_0_4>(
                params,
                field_end!(CloudFilters::CF_CALLBACK_PARAMETERS_0_4, Flags),
            )?
        };
        let details = info::Opened(raw);
        Ok((request, details))
    })();
    let (request, details) = match captured {
        Ok(value) => value,
        Err(error) => {
            unsafe { reject_callback(&*filter, error, FailureKind::Notification, info, params) };
            return;
        }
    };
    filter.opened(request, details);
}

pub unsafe extern "system" fn notify_file_close_completion<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    let Some(filter) = (unsafe { filter_from_info::<T>(info) }) else {
        return;
    };
    let captured = (|| -> Result<_, MetadataError> {
        let request = unsafe { capture_request(info)? };
        let raw = unsafe {
            read_parameter_arm::<CloudFilters::CF_CALLBACK_PARAMETERS_0_5>(
                params,
                field_end!(CloudFilters::CF_CALLBACK_PARAMETERS_0_5, Flags),
            )?
        };
        let details = info::Closed(raw);
        Ok((request, details))
    })();
    let (request, details) = match captured {
        Ok(value) => value,
        Err(error) => {
            unsafe { reject_callback(&*filter, error, FailureKind::Notification, info, params) };
            return;
        }
    };
    filter.closed(request, details);
}

pub unsafe extern "system" fn notify_dehydrate<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    let Some(filter) = (unsafe { filter_from_info::<T>(info) }) else {
        return;
    };
    let captured = (|| -> Result<_, MetadataError> {
        let request = unsafe { capture_request(info)? };
        let raw = unsafe {
            read_parameter_arm::<CloudFilters::CF_CALLBACK_PARAMETERS_0_6>(
                params,
                field_end!(CloudFilters::CF_CALLBACK_PARAMETERS_0_6, Reason),
            )?
        };
        let details = info::Dehydrate(raw);
        Ok((request, details))
    })();
    let (request, details) = match captured {
        Ok(value) => value,
        Err(error) => {
            unsafe { reject_callback(&*filter, error, FailureKind::Dehydrate, info, params) };
            return;
        }
    };
    let route = unsafe { failure_route(FailureKind::Dehydrate, info, params) };
    let ticket = ticket::Dehydrate::new(request.connection_key(), request.transfer_key());
    if let Err(error) = filter.dehydrate(request, ticket, details) {
        if let Some(route) = route {
            report_failure(FailureKind::Dehydrate, route, error);
        }
    }
}

pub unsafe extern "system" fn notify_dehydrate_completion<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    let Some(filter) = (unsafe { filter_from_info::<T>(info) }) else {
        return;
    };
    let captured = (|| -> Result<_, MetadataError> {
        let request = unsafe { capture_request(info)? };
        let raw = unsafe {
            read_parameter_arm::<CloudFilters::CF_CALLBACK_PARAMETERS_0_7>(
                params,
                field_end!(CloudFilters::CF_CALLBACK_PARAMETERS_0_7, Reason),
            )?
        };
        let details = info::Dehydrated(raw);
        Ok((request, details))
    })();
    let (request, details) = match captured {
        Ok(value) => value,
        Err(error) => {
            unsafe { reject_callback(&*filter, error, FailureKind::Notification, info, params) };
            return;
        }
    };
    filter.dehydrated(request, details);
}

pub unsafe extern "system" fn notify_delete<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    let Some(filter) = (unsafe { filter_from_info::<T>(info) }) else {
        return;
    };
    let captured = (|| -> Result<_, MetadataError> {
        let request = unsafe { capture_request(info)? };
        let raw = unsafe {
            read_parameter_arm::<CloudFilters::CF_CALLBACK_PARAMETERS_0_8>(
                params,
                field_end!(CloudFilters::CF_CALLBACK_PARAMETERS_0_8, Flags),
            )?
        };
        let details = info::Delete(raw);
        Ok((request, details))
    })();
    let (request, details) = match captured {
        Ok(value) => value,
        Err(error) => {
            unsafe { reject_callback(&*filter, error, FailureKind::Delete, info, params) };
            return;
        }
    };
    let route = unsafe { failure_route(FailureKind::Delete, info, params) };
    let ticket = ticket::Delete::new(request.connection_key(), request.transfer_key());
    if let Err(error) = filter.delete(request, ticket, details) {
        if let Some(route) = route {
            report_failure(FailureKind::Delete, route, error);
        }
    }
}

pub unsafe extern "system" fn notify_delete_completion<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    let Some(filter) = (unsafe { filter_from_info::<T>(info) }) else {
        return;
    };
    let captured = (|| -> Result<_, MetadataError> {
        let request = unsafe { capture_request(info)? };
        let raw = unsafe {
            read_parameter_arm::<CloudFilters::CF_CALLBACK_PARAMETERS_0_9>(
                params,
                field_end!(CloudFilters::CF_CALLBACK_PARAMETERS_0_9, Flags),
            )?
        };
        let details = info::Deleted(raw);
        Ok((request, details))
    })();
    let (request, details) = match captured {
        Ok(value) => value,
        Err(error) => {
            unsafe { reject_callback(&*filter, error, FailureKind::Notification, info, params) };
            return;
        }
    };
    filter.deleted(request, details);
}

pub unsafe extern "system" fn notify_rename<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    let Some(filter) = (unsafe { filter_from_info::<T>(info) }) else {
        return;
    };
    let captured = (|| -> Result<_, MetadataError> {
        let request = unsafe { capture_request(info)? };
        let raw = unsafe {
            read_parameter_arm::<CloudFilters::CF_CALLBACK_PARAMETERS_0_10>(
                params,
                field_end!(CloudFilters::CF_CALLBACK_PARAMETERS_0_10, TargetPath),
            )?
        };
        let details = unsafe { info::Rename::new(raw, request.volume_utf16())? };
        Ok((request, details))
    })();
    let (request, details) = match captured {
        Ok(value) => value,
        Err(error) => {
            unsafe { reject_callback(&*filter, error, FailureKind::Rename, info, params) };
            return;
        }
    };
    let route = unsafe { failure_route(FailureKind::Rename, info, params) };
    let ticket = ticket::Rename::new(request.connection_key(), request.transfer_key());
    if let Err(error) = filter.rename(request, ticket, details) {
        if let Some(route) = route {
            report_failure(FailureKind::Rename, route, error);
        }
    }
}

pub unsafe extern "system" fn notify_rename_completion<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    let Some(filter) = (unsafe { filter_from_info::<T>(info) }) else {
        return;
    };
    let captured = (|| -> Result<_, MetadataError> {
        let request = unsafe { capture_request(info)? };
        let raw = unsafe {
            read_parameter_arm::<CloudFilters::CF_CALLBACK_PARAMETERS_0_11>(
                params,
                field_end!(CloudFilters::CF_CALLBACK_PARAMETERS_0_11, SourcePath),
            )?
        };
        let details = unsafe { info::Renamed::new(raw, request.volume_utf16())? };
        Ok((request, details))
    })();
    let (request, details) = match captured {
        Ok(value) => value,
        Err(error) => {
            unsafe { reject_callback(&*filter, error, FailureKind::Notification, info, params) };
            return;
        }
    };
    filter.renamed(request, details);
}

unsafe fn filter_from_info<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
) -> Option<super::context::Active<T>> {
    // Resolve only a size-covered opaque token; never dereference its value.
    let token = unsafe {
        callback_field(info, field_end!(CF_CALLBACK_INFO, CallbackContext), |p| {
            ptr::addr_of!((*p).CallbackContext)
        })
    }
    .ok()?;
    super::context::resolve::<T>(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{CResult, CloudErrorKind};
    use std::sync::Arc;
    struct EmptyFilter;
    impl SyncFilter for EmptyFilter {
        fn fetch_data(&self, _: Request, _: ticket::FetchData, _: info::FetchData) -> CResult<()> {
            Err(CloudErrorKind::NotSupported)
        }
    }
    #[test]
    fn callback_context_borrows_without_consuming_weak_ownership() {
        let filter = Arc::new(EmptyFilter);
        let token = super::super::ContextToken::register(&filter);
        let info = CF_CALLBACK_INFO {
            StructSize: size_of::<CF_CALLBACK_INFO>() as u32,
            CallbackContext: token.as_context() as *mut _,
            ..Default::default()
        };
        for _ in 0..3 {
            assert!(unsafe { filter_from_info::<EmptyFilter>(&info) }.is_some());
        }
        assert_eq!(Arc::weak_count(&filter), 1);
        drop(filter);
        for _ in 0..3 {
            assert!(unsafe { filter_from_info::<EmptyFilter>(&info) }.is_none());
        }
    }

    #[test]
    fn callback_after_token_removal_resolves_nothing() {
        let filter = Arc::new(EmptyFilter);
        let token = super::super::ContextToken::register(&filter);
        let info = CF_CALLBACK_INFO {
            StructSize: size_of::<CF_CALLBACK_INFO>() as u32,
            CallbackContext: token.as_context() as *mut _,
            ..Default::default()
        };
        drop(token);
        assert!(unsafe { filter_from_info::<EmptyFilter>(&info) }.is_none());
        assert_eq!(Arc::weak_count(&filter), 0, "registry retained the filter");
    }
}

#[cfg(test)]
#[path = "proxy_owned_tests.rs"]
mod owned_tests;

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

/// Catches Rust unwinding, including a panic payload whose destructor panics.
/// OS-invalid non-null pointers and panic=abort remain outside Rust unwinding.
fn contained<R>(work: impl FnOnce() -> R) -> Option<R> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
        Ok(value) => Some(value),
        Err(payload) => {
            // Dropping an arbitrary user panic payload could unwind again at
            // the ABI boundary. Retain it rather than risk a second unwind.
            std::mem::forget(payload);
            None
        }
    }
}

#[derive(Clone, Copy)]
enum Failure {
    Fetch,
    Validate,
    Placeholders,
    Dehydrate,
    Delete,
    Rename,
    Notification,
}

/// The actual Active guard survives callback execution AND failure completion.
/// Failure completion is attempted once, inside its own panic boundary.
unsafe fn guarded<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
    failure: Failure,
    callback: impl FnOnce(&T) -> crate::error::CResult<()>,
) {
    if info.is_null() || params.is_null() {
        return;
    }
    let outcome = match contained(|| filter_from_info::<T>(info)) {
        Some(Some(filter)) => {
            let outcome = contained(|| callback(&filter))
                .unwrap_or(Err(crate::error::CloudErrorKind::Unsuccessful));
            if let Err(error) = outcome {
                report_failure(info, params, failure, error);
            }
            // Keep the original filter/context active through native reporting.
            // A last user filter Arc may run arbitrary Drop code here as well.
            let _ = contained(|| drop(filter));
            return;
        }
        Some(None) => return, // Original registration was already removed.
        None => crate::error::CloudErrorKind::Unsuccessful,
    };
    report_failure(info, params, failure, outcome);
}

unsafe fn report_failure(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
    failure: Failure,
    error: crate::error::CloudErrorKind,
) {
    let _ = contained(|| {
        // Read only the native numeric completion keys; Request construction
        // may have been the operation which panicked.
        let connection = (*info).ConnectionKey.0;
        let transfer = (*info).TransferKey;
        let result = match failure {
            Failure::Fetch => {
                let range = (*params).Anonymous.FetchData;
                command::Write::fail_range(
                    connection,
                    transfer,
                    error,
                    range.RequiredFileOffset,
                    range.RequiredLength,
                )
            }
            Failure::Validate => command::Validate::fail(connection, transfer, error),
            Failure::Placeholders => command::CreatePlaceholders::fail(connection, transfer, error),
            Failure::Dehydrate => command::Dehydrate::fail(connection, transfer, error),
            Failure::Delete => command::Delete::fail(connection, transfer, error),
            Failure::Rename => command::Rename::fail(connection, transfer, error),
            Failure::Notification => {
                log::error!("Native notification callback panicked");
                return;
            }
        };
        if let Err(error) = result {
            log::error!("Cannot report callback failure: {error}");
        }
    });
}

pub unsafe extern "system" fn fetch_data<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    guarded::<T>(info, params, Failure::Fetch, |filter| {
        let request = Request::new(*info);
        let ticket = ticket::FetchData::new(request.connection_key(), request.transfer_key());
        filter.fetch_data(
            request,
            ticket,
            info::FetchData((*params).Anonymous.FetchData),
        )
    });
}

pub unsafe extern "system" fn validate_data<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    guarded::<T>(info, params, Failure::Validate, |filter| {
        let request = Request::new(*info);
        let ticket = ticket::ValidateData::new(request.connection_key(), request.transfer_key());
        filter.validate_data(
            request,
            ticket,
            info::ValidateData((*params).Anonymous.ValidateData),
        )
    });
}

pub unsafe extern "system" fn fetch_placeholders<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    guarded::<T>(info, params, Failure::Placeholders, |filter| {
        let request = Request::new(*info);
        let ticket =
            ticket::FetchPlaceholders::new(request.connection_key(), request.transfer_key());
        filter.fetch_placeholders(
            request,
            ticket,
            info::FetchPlaceholders((*params).Anonymous.FetchPlaceholders),
        )
    });
}

pub unsafe extern "system" fn notify_dehydrate<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    guarded::<T>(info, params, Failure::Dehydrate, |filter| {
        let request = Request::new(*info);
        let ticket = ticket::Dehydrate::new(request.connection_key(), request.transfer_key());
        filter.dehydrate(
            request,
            ticket,
            info::Dehydrate((*params).Anonymous.Dehydrate),
        )
    });
}

pub unsafe extern "system" fn notify_delete<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    guarded::<T>(info, params, Failure::Delete, |filter| {
        let request = Request::new(*info);
        let ticket = ticket::Delete::new(request.connection_key(), request.transfer_key());
        filter.delete(request, ticket, info::Delete((*params).Anonymous.Delete))
    });
}

pub unsafe extern "system" fn cancel_fetch_data<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    guarded::<T>(info, params, Failure::Notification, |filter| {
        filter.cancel_fetch_data(
            Request::new(*info),
            info::CancelFetchData((*params).Anonymous.Cancel),
        );
        Ok(())
    });
}

pub unsafe extern "system" fn cancel_fetch_placeholders<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    guarded::<T>(info, params, Failure::Notification, |filter| {
        filter.cancel_fetch_placeholders(
            Request::new(*info),
            info::CancelFetchPlaceholders((*params).Anonymous.Cancel),
        );
        Ok(())
    });
}

pub unsafe extern "system" fn notify_file_open_completion<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    guarded::<T>(info, params, Failure::Notification, |filter| {
        filter.opened(
            Request::new(*info),
            info::Opened((*params).Anonymous.OpenCompletion),
        );
        Ok(())
    });
}

pub unsafe extern "system" fn notify_file_close_completion<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    guarded::<T>(info, params, Failure::Notification, |filter| {
        filter.closed(
            Request::new(*info),
            info::Closed((*params).Anonymous.CloseCompletion),
        );
        Ok(())
    });
}

pub unsafe extern "system" fn notify_dehydrate_completion<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    guarded::<T>(info, params, Failure::Notification, |filter| {
        filter.dehydrated(
            Request::new(*info),
            info::Dehydrated((*params).Anonymous.DehydrateCompletion),
        );
        Ok(())
    });
}

pub unsafe extern "system" fn notify_delete_completion<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    guarded::<T>(info, params, Failure::Notification, |filter| {
        filter.deleted(
            Request::new(*info),
            info::Deleted((*params).Anonymous.DeleteCompletion),
        );
        Ok(())
    });
}

pub unsafe extern "system" fn notify_rename<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    guarded::<T>(info, params, Failure::Rename, |filter| {
        let request = Request::new(*info);
        let ticket = ticket::Rename::new(request.connection_key(), request.transfer_key());
        let details = info::Rename((*params).Anonymous.Rename, request.volume_letter());
        filter.rename(request, ticket, details)
    });
}

pub unsafe extern "system" fn notify_rename_completion<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
    params: *const CF_CALLBACK_PARAMETERS,
) {
    guarded::<T>(info, params, Failure::Notification, |filter| {
        let request = Request::new(*info);
        let details = info::Renamed(
            (*params).Anonymous.RenameCompletion,
            request.volume_letter(),
        );
        filter.renamed(request, details);
        Ok(())
    });
}

unsafe fn filter_from_info<T: SyncFilter + 'static>(
    info: *const CF_CALLBACK_INFO,
) -> Option<super::context::Active<T>> {
    // CallbackContext is an opaque registry token, never dereferenced: native
    // disconnect does not drain dispatched callbacks, so late callbacks must be
    // able to observe a removed registration safely.
    super::context::resolve::<T>((*info).CallbackContext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containment_preserves_results_and_catches_panics() {
        assert_eq!(contained(|| 17), Some(17));
        assert_eq!(contained(|| panic!("injected callback panic")), None::<()>);
        struct PanicOnDrop;
        impl Drop for PanicOnDrop {
            fn drop(&mut self) {
                panic!("panic payload must not be dropped at ABI boundary");
            }
        }
        assert_eq!(contained(|| std::panic::panic_any(PanicOnDrop)), None::<()>);
    }

    #[test]
    fn null_callback_arguments_do_not_call_filter_or_failure_completion() {
        let never = |_: &EmptyFilter| -> crate::error::CResult<()> {
            panic!("null callback was dispatched")
        };
        unsafe {
            guarded::<EmptyFilter>(std::ptr::null(), std::ptr::null(), Failure::Fetch, never);
            let info = CF_CALLBACK_INFO::default();
            guarded::<EmptyFilter>(&info, std::ptr::null(), Failure::Fetch, never);
            let params = CF_CALLBACK_PARAMETERS::default();
            guarded::<EmptyFilter>(std::ptr::null(), &params, Failure::Notification, never);
        }
    }

    use crate::error::{CResult, CloudErrorKind};
    use std::sync::Arc;
    struct EmptyFilter;
    impl SyncFilter for EmptyFilter {
        fn fetch_data(&self, _: Request, _: ticket::FetchData, _: info::FetchData) -> CResult<()> {
            Err(CloudErrorKind::NotSupported)
        }
    }
    #[test]
    fn panicking_notification_releases_its_original_active_callback_guard() {
        let filter = Arc::new(EmptyFilter);
        let token = super::super::ContextToken::register(&filter);
        let active = token.active();
        let info = CF_CALLBACK_INFO {
            CallbackContext: token.as_context() as *mut _,
            ..Default::default()
        };
        let params = CF_CALLBACK_PARAMETERS::default();
        unsafe {
            guarded::<EmptyFilter>(&info, &params, Failure::Notification, |_| {
                panic!("injected notification panic")
            });
        }
        assert_eq!(active.load(std::sync::atomic::Ordering::Acquire), 0);
        assert_eq!(Arc::strong_count(&filter), 1);
    }

    #[test]
    fn callback_context_borrows_without_consuming_weak_ownership() {
        let filter = Arc::new(EmptyFilter);
        let token = super::super::ContextToken::register(&filter);
        let info = CF_CALLBACK_INFO {
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
            CallbackContext: token.as_context() as *mut _,
            ..Default::default()
        };
        drop(token);
        assert!(unsafe { filter_from_info::<EmptyFilter>(&info) }.is_none());
        assert_eq!(Arc::weak_count(&filter), 0, "registry retained the filter");
    }
}

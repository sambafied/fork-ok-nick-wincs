//! Pure callback-boundary fixtures. These tests never register a root, invoke
//! CfExecute, or dispatch a fallible native callback with acknowledged keys.

use super::*;
use crate::{
    error::{CResult, CloudErrorKind},
    filter::{AsyncBridge, ContextToken, Filter, MetadataError},
    utility::LocalBoxFuture,
};
use std::{
    ffi::OsString,
    mem::{offset_of, size_of},
    path::PathBuf,
    ptr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use windows::{
    core::PCWSTR,
    Win32::Storage::CloudFilters::{
        CF_CALLBACK_PARAMETERS_0_1, CF_CONNECTION_KEY, CF_PROCESS_INFO,
    },
};

#[derive(Default)]
struct CountedFilter {
    requests: Mutex<Vec<Request>>,
    errors: Mutex<Vec<MetadataError>>,
}

impl CountedFilter {
    fn capture(&self, request: Request) {
        self.requests.lock().unwrap().push(request);
    }

    fn assert_rejected(&self, count: usize, expected: MetadataError) {
        assert_eq!(self.requests.lock().unwrap().len(), 0);
        let errors = self.errors.lock().unwrap();
        assert_eq!(errors.len(), count);
        assert_eq!(errors.last().copied(), Some(expected));
    }
}

impl SyncFilter for CountedFilter {
    fn metadata_error(&self, error: MetadataError) {
        self.errors.lock().unwrap().push(error);
    }

    fn fetch_data(&self, _: Request, _: ticket::FetchData, _: info::FetchData) -> CResult<()> {
        // This implementation must never be reached by these fixtures.
        self.requests.lock().unwrap().clear();
        Err(CloudErrorKind::NotSupported)
    }

    fn cancel_fetch_data(&self, request: Request, _: info::CancelFetchData) {
        self.capture(request);
    }

    fn cancel_fetch_placeholders(&self, request: Request, _: info::CancelFetchPlaceholders) {
        self.capture(request);
    }

    fn opened(&self, request: Request, _: info::Opened) {
        self.capture(request);
    }

    fn closed(&self, request: Request, _: info::Closed) {
        self.capture(request);
    }

    fn dehydrated(&self, request: Request, _: info::Dehydrated) {
        self.capture(request);
    }

    fn deleted(&self, request: Request, _: info::Deleted) {
        self.capture(request);
    }

    fn renamed(&self, request: Request, _: info::Renamed) {
        self.capture(request);
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// Every pointer in `raw` points to storage owned by this fixture.
struct NativeFixture {
    raw: CF_CALLBACK_INFO,
    guid: Vec<u16>,
    dos: Vec<u16>,
    path: Vec<u16>,
    file_blob: Vec<u8>,
    root_blob: Vec<u8>,
    process: Box<CF_PROCESS_INFO>,
    image: Vec<u16>,
    package: Vec<u16>,
    application: Vec<u16>,
    command: Vec<u16>,
}

impl NativeFixture {
    fn new(context: *mut std::ffi::c_void) -> Self {
        let guid = wide(r"\\?\Volume{fixture}");
        let dos = wide("Q:");
        let path = wide(r"owned\original.bin");
        let file_blob = vec![1, 3, 5, 7];
        let root_blob = vec![2, 4, 6];
        let image = wide(r"Q:\apps\original.exe");
        let package = wide("original-package");
        let application = wide("original-application");
        let command = wide("original.exe --original");
        let process = Box::new(CF_PROCESS_INFO {
            StructSize: size_of::<CF_PROCESS_INFO>() as u32,
            ProcessId: 271,
            SessionId: 19,
            ImagePath: PCWSTR(image.as_ptr()),
            PackageName: PCWSTR(package.as_ptr()),
            ApplicationId: PCWSTR(application.as_ptr()),
            CommandLine: PCWSTR(command.as_ptr()),
            ..Default::default()
        });
        let raw = CF_CALLBACK_INFO {
            StructSize: size_of::<CF_CALLBACK_INFO>() as u32,
            CallbackContext: context,
            VolumeGuidName: PCWSTR(guid.as_ptr()),
            VolumeDosName: PCWSTR(dos.as_ptr()),
            NormalizedPath: PCWSTR(path.as_ptr()),
            FileIdentity: file_blob.as_ptr().cast(),
            FileIdentityLength: file_blob.len() as u32,
            SyncRootIdentity: root_blob.as_ptr().cast(),
            SyncRootIdentityLength: root_blob.len() as u32,
            ProcessInfo: ptr::from_ref(process.as_ref()),
            ConnectionKey: CF_CONNECTION_KEY(101),
            TransferKey: 202,
            FileSize: 303,
            FileId: 404,
            SyncRootFileId: 505,
            VolumeSerialNumber: 606,
            PriorityHint: 7,
            ..Default::default()
        };
        Self {
            raw,
            guid,
            dos,
            path,
            file_blob,
            root_blob,
            process,
            image,
            package,
            application,
            command,
        }
    }

    fn overwrite_native_storage(&mut self) {
        for text in [
            &mut self.guid,
            &mut self.dos,
            &mut self.path,
            &mut self.image,
            &mut self.package,
            &mut self.application,
            &mut self.command,
        ] {
            text.fill(b'X' as u16);
        }
        self.file_blob.fill(0xEE);
        self.root_blob.fill(0xDD);
        self.process.ProcessId = 999;
        self.process.SessionId = 888;
        self.raw.FileSize = 777;
        self.raw.ConnectionKey = CF_CONNECTION_KEY(666);
        self.raw.TransferKey = 555;
    }
}

fn full_params() -> CF_CALLBACK_PARAMETERS {
    CF_CALLBACK_PARAMETERS {
        ParamSize: size_of::<CF_CALLBACK_PARAMETERS>() as u32,
        ..Default::default()
    }
}

type Notification =
    unsafe extern "system" fn(*const CF_CALLBACK_INFO, *const CF_CALLBACK_PARAMETERS);

fn notifications() -> [Notification; 7] {
    [
        cancel_fetch_data::<CountedFilter>,
        cancel_fetch_placeholders::<CountedFilter>,
        notify_file_open_completion::<CountedFilter>,
        notify_file_close_completion::<CountedFilter>,
        notify_dehydrate_completion::<CountedFilter>,
        notify_delete_completion::<CountedFilter>,
        notify_rename_completion::<CountedFilter>,
    ]
}

#[test]
fn opened_captures_request_before_native_storage_is_mutated_and_freed() {
    let filter = Arc::new(CountedFilter::default());
    let token = ContextToken::register(&filter);
    let mut native = NativeFixture::new(token.as_context() as *mut _);
    let params = full_params();

    unsafe {
        notify_file_open_completion::<CountedFilter>(&native.raw, &params);
    }

    // Do not inspect any captured metadata until all native storage is gone.
    native.overwrite_native_storage();
    drop(native);

    assert!(filter.errors.lock().unwrap().is_empty());
    let mut requests = filter.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let request = requests.pop().unwrap();
    assert_eq!(
        request.volume_guid_path(),
        OsString::from(r"\\?\Volume{fixture}")
    );
    assert_eq!(request.volume_letter(), OsString::from("Q:"));
    assert_eq!(request.path(), PathBuf::from(r"Q:\owned\original.bin"));
    assert_eq!(request.file_blob(), &[1, 3, 5, 7]);
    assert_eq!(request.register_blob(), &[2, 4, 6]);
    assert_eq!(request.connection_key(), 101);
    assert_eq!(request.transfer_key(), 202);
    assert_eq!(request.file_size(), 303);
    assert_eq!(request.file_id(), 404);
    assert_eq!(request.sync_root_file_id(), 505);
    assert_eq!(request.volume_serial_number(), 606);
    assert_eq!(request.priority_hint(), 7);

    let process = request.process().unwrap();
    assert_eq!(process.id(), 271);
    assert_eq!(process.session_id(), Some(19));
    assert_eq!(process.path(), Some(PathBuf::from(r"Q:\apps\original.exe")));
    assert_eq!(process.name(), Some(OsString::from("original-package")));
    assert_eq!(
        process.application_id(),
        Some(OsString::from("original-application"))
    );
    assert_eq!(
        process.command_line(),
        Some(OsString::from("original.exe --original"))
    );
    let identity = request.process_identity().unwrap();
    assert_eq!(identity.id(), 271);
    assert_eq!(identity.session_id(), 19);
}

#[test]
fn context_must_be_fully_covered_before_registry_lookup() {
    let filter = Arc::new(CountedFilter::default());
    let token = ContextToken::register(&filter);
    let mut native = NativeFixture::new(token.as_context() as *mut _);
    let context_end =
        offset_of!(CF_CALLBACK_INFO, CallbackContext) + size_of::<*mut std::ffi::c_void>();

    native.raw.StructSize = (context_end - 1) as u32;
    assert!(unsafe { filter_from_info::<CountedFilter>(&native.raw) }.is_none());

    native.raw.StructSize = context_end as u32;
    assert!(unsafe { filter_from_info::<CountedFilter>(&native.raw) }.is_some());

    native.raw.CallbackContext = ptr::null_mut();
    assert!(unsafe { filter_from_info::<CountedFilter>(&native.raw) }.is_none());

    native.raw.CallbackContext = token.as_context() as *mut _;
    drop(token);
    assert!(unsafe { filter_from_info::<CountedFilter>(&native.raw) }.is_none());
    assert!(filter.requests.lock().unwrap().is_empty());
}

#[test]
fn notifications_with_only_a_size_header_do_not_dispatch() {
    let header = Box::new(size_of::<u32>() as u32);
    let native = ptr::from_ref(header.as_ref()).cast::<CF_CALLBACK_INFO>();
    let params = full_params();

    for callback in notifications() {
        unsafe { callback(native, &params) };
    }
    // The allocation really contains only the header. No context or full
    // callback storage is available behind it.
    assert!(unsafe { filter_from_info::<CountedFilter>(native) }.is_none());
}

#[test]
fn notifications_with_no_context_do_not_dispatch() {
    let filter = Arc::new(CountedFilter::default());
    let native = NativeFixture::new(ptr::null_mut());
    let params = full_params();
    for callback in notifications() {
        unsafe { callback(&native.raw, &params) };
    }
    assert!(filter.requests.lock().unwrap().is_empty());
    assert!(filter.errors.lock().unwrap().is_empty());
}

#[test]
fn resolved_context_reports_truncated_callback_before_normal_dispatch() {
    let filter = Arc::new(CountedFilter::default());
    let token = ContextToken::register(&filter);
    let mut native = NativeFixture::new(token.as_context() as *mut _);
    native.raw.StructSize =
        (offset_of!(CF_CALLBACK_INFO, CallbackContext) + size_of::<*mut std::ffi::c_void>()) as u32;
    let params = full_params();

    for (index, callback) in notifications().into_iter().enumerate() {
        unsafe { callback(&native.raw, &params) };
        filter.assert_rejected(index + 1, MetadataError::TruncatedCallback);
    }
}

#[test]
fn opened_rejects_missing_mandatory_fields_and_invalid_blob_before_dispatch() {
    let filter = Arc::new(CountedFilter::default());
    let token = ContextToken::register(&filter);
    let mut native = NativeFixture::new(token.as_context() as *mut _);
    let params = full_params();

    let guid = native.raw.VolumeGuidName;
    native.raw.VolumeGuidName = PCWSTR::null();
    unsafe { notify_file_open_completion::<CountedFilter>(&native.raw, &params) };
    filter.assert_rejected(1, MetadataError::InvalidString);
    native.raw.VolumeGuidName = guid;

    let dos = native.raw.VolumeDosName;
    native.raw.VolumeDosName = PCWSTR::null();
    unsafe { notify_file_open_completion::<CountedFilter>(&native.raw, &params) };
    filter.assert_rejected(2, MetadataError::InvalidString);
    native.raw.VolumeDosName = dos;

    let path = native.raw.NormalizedPath;
    native.raw.NormalizedPath = PCWSTR::null();
    unsafe { notify_file_open_completion::<CountedFilter>(&native.raw, &params) };
    filter.assert_rejected(3, MetadataError::InvalidString);
    native.raw.NormalizedPath = path;

    native.raw.FileIdentity = ptr::null();
    // Preserve the nonzero declared extent: this must not become an empty blob.
    unsafe { notify_file_open_completion::<CountedFilter>(&native.raw, &params) };
    filter.assert_rejected(4, MetadataError::InvalidBlob);
}

#[test]
fn notification_parameter_header_is_rejected_before_union_dispatch() {
    let filter = Arc::new(CountedFilter::default());
    let token = ContextToken::register(&filter);
    let native = NativeFixture::new(token.as_context() as *mut _);
    let header = Box::new(size_of::<u32>() as u32);
    let params = ptr::from_ref(header.as_ref()).cast::<CF_CALLBACK_PARAMETERS>();

    for (index, callback) in notifications().into_iter().enumerate() {
        unsafe { callback(&native.raw, params) };
        filter.assert_rejected(index + 1, MetadataError::TruncatedParameters);
    }
}

#[test]
fn notification_failure_routing_never_reads_or_acknowledges_metadata() {
    // Null pointers intentionally provide no readable header or keys.
    assert!(
        unsafe { failure_route(FailureKind::Notification, ptr::null(), ptr::null()) }.is_none()
    );

    let native = NativeFixture::new(ptr::null_mut());
    let params = full_params();
    assert!(unsafe { failure_route(FailureKind::Notification, &native.raw, &params) }.is_none());
}

#[test]
fn fetch_failure_route_preserves_original_keys_and_exact_eof_range() {
    let native = NativeFixture::new(ptr::null_mut());
    let mut params = full_params();
    params.Anonymous.FetchData = CF_CALLBACK_PARAMETERS_0_1 {
        RequiredFileOffset: 37,
        RequiredLength: -1,
        ..Default::default()
    };

    let route = unsafe { failure_route(FailureKind::FetchData, &native.raw, &params) }
        .expect("complete original keys and required range must be routable");

    assert_eq!(route.connection_key, 101);
    assert_eq!(route.transfer_key, 202);
    assert_eq!(route.range, Some((37, -1)));
}

#[test]
fn fetch_failure_route_requires_the_complete_original_range() {
    let native = NativeFixture::new(ptr::null_mut());
    let mut params = full_params();
    params.Anonymous.FetchData = CF_CALLBACK_PARAMETERS_0_1 {
        RequiredFileOffset: 37,
        RequiredLength: 4096,
        ..Default::default()
    };
    let range_end = offset_of!(CF_CALLBACK_PARAMETERS, Anonymous)
        + offset_of!(CF_CALLBACK_PARAMETERS_0_1, RequiredLength)
        + size_of::<i64>();

    params.ParamSize = (range_end - 1) as u32;
    assert!(unsafe { failure_route(FailureKind::FetchData, &native.raw, &params) }.is_none());
    assert!(unsafe { failure_route(FailureKind::FetchData, &native.raw, ptr::null()) }.is_none());

    params.ParamSize = range_end as u32;
    let route = unsafe { failure_route(FailureKind::FetchData, &native.raw, &params) }
        .expect("the exact required range prefix must be sufficient");
    assert_eq!(route.range, Some((37, 4096)));
}

#[test]
fn failure_routes_require_complete_native_keys_without_fabricating_zeroes() {
    let mut native = NativeFixture::new(ptr::null_mut());
    let params = full_params();
    let keys_end = offset_of!(CF_CALLBACK_INFO, TransferKey) + size_of::<i64>();

    native.raw.StructSize = (keys_end - 1) as u32;
    for kind in [
        FailureKind::FetchData,
        FailureKind::ValidateData,
        FailureKind::FetchPlaceholders,
        FailureKind::Dehydrate,
        FailureKind::Delete,
        FailureKind::Rename,
    ] {
        assert!(unsafe { failure_route(kind, &native.raw, &params) }.is_none());
    }

    native.raw.StructSize = keys_end as u32;
    for kind in [
        FailureKind::ValidateData,
        FailureKind::FetchPlaceholders,
        FailureKind::Dehydrate,
        FailureKind::Delete,
        FailureKind::Rename,
    ] {
        // Non-fetch failure acknowledgements require no fetched range and must
        // not demand or read a parameter buffer just to retain original keys.
        let route = unsafe { failure_route(kind, &native.raw, ptr::null()) }
            .expect("covered original keys must remain usable");
        assert_eq!(route.connection_key, 101);
        assert_eq!(route.transfer_key, 202);
        assert_eq!(route.range, None);
    }
}

struct AsyncCounted {
    errors: Arc<Mutex<Vec<MetadataError>>>,
}

impl Filter for AsyncCounted {
    fn metadata_error(&self, error: MetadataError) {
        self.errors.lock().unwrap().push(error);
    }

    fn fetch_data(
        &self,
        _: Request,
        _: ticket::FetchData,
        _: info::FetchData,
    ) -> impl std::future::Future<Output = CResult<()>> {
        async { Err(CloudErrorKind::NotSupported) }
    }
}

#[test]
fn async_bridge_reports_metadata_errors_without_running_or_polling_executor() {
    let errors = Arc::new(Mutex::new(Vec::new()));
    let executor_calls = Arc::new(AtomicUsize::new(0));
    let observed_calls = Arc::clone(&executor_calls);
    let bridge = AsyncBridge::new(
        AsyncCounted {
            errors: Arc::clone(&errors),
        },
        move |_future: LocalBoxFuture<'_, ()>| {
            observed_calls.fetch_add(1, Ordering::SeqCst);
            // Intentionally never poll.
        },
    );

    SyncFilter::metadata_error(&bridge, MetadataError::InvalidBlob);
    SyncFilter::metadata_error(&bridge, MetadataError::TruncatedParameters);

    assert_eq!(executor_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        *errors.lock().unwrap(),
        [
            MetadataError::InvalidBlob,
            MetadataError::TruncatedParameters,
        ]
    );
}

# Fallible owned callback metadata before asynchronous dispatch

Development fork#3 / Desktop#532/#56/#448, R13-CACHE-01, AEM-005/009/010/014.
Base a8db8d8920be615c575b043d1167a9ada148ed2c. PR32 supplied only owned scalar
PID/session metadata; this follow-up must remove all consumed native metadata
pointers from Request, Process, population patterns and rename information.
PR31/issue5 original operation-context owner and retained worktree are untouched.
No Desktop dependency pin, root, registration, package, startup or trust change.

## Owned representation and public contract

Use a private metadata helper with redacted fixed MetadataError variants:
TruncatedCallback, TruncatedParameters, InvalidString, InvalidBlob,
InvalidScalar, Allocation. No errors or default Debug expose paths, blobs,
process IDs, package identities or command lines. Errors describe validation
only and confer no operation, caller, process or cleanup authority.

Request captures the consumed scalar connection/transfer keys, serial/root/file
IDs, nonnegative file size and priority, plus owned UTF-16 strings and byte blobs
at callback entry. It retains no CF_CALLBACK_INFO or CF_PROCESS_INFO pointer.
Require a fully size-covered callback before copying its ABI struct; reject
unsupported/truncated size before reading any pointer payload. This wrapper's
new owned-capture path refuses an older incomplete callback structure rather
than copying its absent tail. Mandatory volume GUID/DOS/normalized strings
refuse null; process/package/application/command strings are nullable. Scan at
most32768 UTF-16 units including the terminator. File identity is at most4096
bytes and sync-root identity65536. Null plus zero-length blob is an empty Vec;
null with nonzero length, oversize, unterminated or overflowed metadata refuses.
Use fallible reservations for snapshot storage. Getter/cloned Process/OsString/
PathBuf allocations retain their existing allocation behavior; the allocation
refusal promise covers snapshot construction. A normal snapshot plus one
owned pattern/rename path is bounded below1MiB; this is an allocation bound,
not complete caller/native-frame resource funding or Source readiness.
Native callback pointers must refer to readable SDK buffers for their declared
extent; Rust validation cannot make arbitrary forged non-null addresses safe.

Request path/blob/scalar getters keep their existing return shapes and derive
only from owned data. Process becomes an owned nullable value: Request::process
returns Option<Process> (cloned owned snapshot); Process::name/application_id
return Option<OsString>, session_id returns Option<u32>, path/command_line retain
Option shapes. CommandLine uses CommandLine, never ImagePath. Missing/UNKNOWN
metadata remains absent. Scalar process_identity remains optional and requires
nonzero PID plus size-covered SessionId; zero session is valid DATA. Default
Request and Process Debug are redacted. These nullable API changes require
explicit downstream source migration before any product pin adoption.

Population patterns and rename/source paths copy before async dispatch.
The absent optional population pattern means the SDK's unrestricted '*' pattern;
existing String/glob return shapes remain. Rename/renamed keep public path/flag
semantics, storing owned paths and flags instead of pointer-bearing union arms.
The other scalar-only info wrappers keep their existing APIs.

## Admission and completion behavior

The proxy validates info/parameter pointers and size before copying each ABI
structure/union arm. Resolve a filter only after StructSize covers CallbackContext.
Extract failure-routing connection/transfer keys and the required fetch range
only behind their own field-end guards. A null/truncated structure without those
original values cannot authorize an acknowledgement: refuse normal dispatch,
notify a safely resolved filter when available, and never invent keys or a
zero-length range. When those original values are available, a failed owned
snapshot is reported through the existing connection/transfer/range failure
route before consumer effects. Completion/cancel notifications have no invented
acknowledgement. Existing executor routing still uses CF_REQUEST_KEY_DEFAULT;
full callback RequestKey/operation-context fidelity remains issue5/PR31 and is
not qualified here. No competing OpContext or retained-owner adoption is added.

Parameter admission uses the actual union offset plus the end of every consumed
field, not the whole CF_CALLBACK_PARAMETERS size. Copy only covered bytes into
the appropriate default-initialized ABI arm. Cancel-population consumes only
flags; it must not read an unavailable cancel-fetch range. Respect ABI alignment
and preserve legitimate CF_EOF == -1 operation lengths. Full known callback-info
prefix admission intentionally refuses incomplete old layouts and accepts larger
future sizes without reading their unknown tails. Process metadata is read field
by field: preserve available old PID/image/package/application fields while later
CommandLine/SessionId remain absent. No all-Windows-version compatibility claim.

Report metadata failures through redacted hooks on both SyncFilter and Filter,
forwarded by AsyncBridge, so a future publisher can invalidate root health rather
than silently treating a dropped event as complete coverage. No healthy-stream,
process ownership or cleanup permission is inferred from the default hook.
General callback panic containment and async future/resource lifetime remain
issue4/whole-native gates; this patch does not claim to repair them.

## Required regression and delivery evidence

Tests mutate and destroy backing strings/blobs/process/parameter allocations
before the first accessor and before moving the owned Request/info to another
thread. Cover null/empty/UNKNOWN, older process sizes, truncated callback and
parameter sizes (including non-null pointer sentinels that must not be read),
length caps/overflow, negative file size, distinct image/command-line data,
scalar identity preservation, redacted Debug, rename flags and patterns.
Malformed metadata must refuse before normal consumer dispatch and use the
correct original failure route without manufacturing native registration.

Main owns integration, parser/meaningful Windows compilation and exact unit
execution, independent source review/rechecks, signed immediate pushes and PR.
Patch workers return code only and exit. Current compile slot remains with
Doctor80; no competing Cargo, installs or native fixtures. New tests remain
NOT_RUN until actually executed. Native callbacks, installed/BOTH-PC acceptance,
actual-use publication, default cleanup and full issue3/4/10 stay unqualified.
Original139 is hash-only;141 is untouched. Rejected old fork-cache cleanup is
not retried, bypassed or used as authority to remove its worktree/evidence.

Primary contracts:
- https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_callback_info
- https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_process_info
- https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_callback_parameters

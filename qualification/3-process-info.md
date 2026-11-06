# Owned optional callback process identity for cache-use observation

Development: fork#3, Desktop#532/#56/#448, R13-CACHE-01,
AEM-005/009/010/014. Base7ca3132899b8bc97e279e1e072442ca9ec16b449.
This supplies one prerequisite for the missing actual-use publisher, not that
publisher or default cleanup activation. Existing PR31/#5 and its original
retained worktree/owners remain untouched; no adoption or custody transfer.

Independent review corrected the initial contract's flag interpretation:
Session's stored default flags are NONE, but connect_retained already always
adds REQUIRE_PROCESS_INFO and REQUIRE_FULL_FILE_PATH at the actual native call.
The proposed builder was therefore redundant. It and its two new private-flag
tests were removed; src/root/session.rs is byte-identical to the base. Existing
effective native defaults, connect/retained ownership and legacy tests survive.
No new connection flag, native lifecycle or registration is introduced.

Request::process_identity returns an optional owned scalar PID/session snapshot,
never a CF_PROCESS_INFO wrapper or native string pointer. Capture occurs inside
Request::new while original callback metadata is valid, before any dispatch;
later accessor calls only copy the saved owned Option. Callback StructSize must
include ProcessInfo before reading its pointer. Missing/null/undersized process
metadata and zero PID produce None. Only size-covered scalar fields are read;
no input pointer survives in the snapshot. Debug is redacted. No process string,
allocation, process open, source authority, native owner or cleanup permission.
PID/session DATA cannot establish process birth, matching image, consumer
classification or continuing lifetime. Unknown attribution must preserve bytes.

Legacy process/string APIs remain unchanged. Full owned callback metadata,
async borrowing and general FFI panic containment remain separate open#3/#4
work. This patch does not qualify them or PR31 operation context changes.

Six deterministic regression definitions cover a truncated callback whose
non-null pointer must not be dereferenced; absent process; incomplete process
headers/SessionId; zero PID; valid session0; and the owned value surviving native
backing mutation/destruction before its first accessor, with Debug redaction.
No native root/registration, service, installation or identity exposure is used.
Main owns exact Windows compilation/tests, independent review, signed immediate
pushes, PR and cleanup. Tests remain NOT_RUN until executed. Product Cargo pins
need separate reviewed integration; source/unit success is not genuine callback,
native/installed/BOTH-PC or full automatic cache acceptance.

History: the initial contract was signed/pushed atf1c4a42 before code1f9edbef.
Full independent review of1f9edbef found the redundant flag claim and otherwise
found the owned snapshot coherent. This correction preserves that review history;
a fresh repair recheck and all six current Windows bodies remain required.

Primary API contracts:
- https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_callback_info
- https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_process_info

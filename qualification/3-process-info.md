# Optional callback process identity for cache-use observation

Development: fork#3, Desktop#532/#56/#448, R13-CACHE-01,
AEM-005/009/010/014. Base7ca3132899b8bc97e279e1e072442ca9ec16b449.
This supplies prerequisites for the missing actual-use publisher, not that
publisher or default cleanup activation. Existing PR31/#5 and its original
retained worktree/owners remain untouched; no adoption or custody transfer.

Add an explicit Session builder option requesting CF_CONNECT_FLAG_REQUIRE_PROCESS_INFO.
Keep Session::new/default flags unchanged. Existing legacy connect and retained
connect share those actual flags, without a new native lifecycle or registration.

Add Request::process_identity returning an optional owned scalar PID/session
snapshot, never a CF_PROCESS_INFO wrapper or native string pointer. Check that
the callback StructSize includes ProcessInfo before inspecting its pointer;
missing/null/undersized process metadata and zero PID return None. Read only
fields whose declared native struct size includes them; retain no input pointer.
No ImagePath/PackageName/ApplicationId/CommandLine parsing, allocation, logging,
process opening, source authority, native owner or cleanup permission is added.
Default Debug of the new snapshot is redacted. PID/session DATA cannot establish
process birth, matching image, consumer classification or continuing lifetime.
The future caller must conservatively preserve bytes for unknown attribution.

Keep the legacy process API unchanged in this bounded patch; full owned callback
metadata, async borrowing, general FFI panic containment and existing unsafe
convenience methods remain separate open#3/#4 work. This patch does not qualify
those whole issues or PR31's operation context changes.

Generate deterministic unit regressions covering default/explicit flags without
connecting; truncated callback metadata that must not dereference a supplied
non-null pointer; null process; incomplete process headers/SessionId; zero PID;
and valid owned scalar data surviving mutation/destruction of the native source.
No native root/registration, service, installation or process metadata exposure.
Main owns compilation/tests/review, signed immediate pushes, PR and cleanup.
Full exact-head independent review and actual Windows compilation/tests precede
eligible merge. Product Cargo pin changes require their own reviewed integration;
source/library success is not genuine CFAPI callback/native/BOTH-PC acceptance.

Primary API contracts:
- https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ne-cfapi-cf_connect_flags
- https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_callback_info
- https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_process_info

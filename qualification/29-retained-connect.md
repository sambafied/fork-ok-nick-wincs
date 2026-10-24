# Retained connect and shutdown ownership contract

Development, maintained fork #29; necessary dependency for desktop-client-adobe
W29 / F09 / #12 / AEM-011,013 and draft PR189. Original single-use W29 owner
implements this whole bounded dependency contract. Base: actual normal fork main
e7865a7d49f94b22bf4e8d7ddbffa1fbc7e1bbe1. Root coordinator approved the exact
source/placement/Git proposal at desktop4d885cd5 before this worktree/source.

Original source failure: Session::connect creates a native connection then
attempts watcher startup; a failed native rollback after watcher-spawn error
returns only Err, dropping the actual key/context ownership. Filter references
retain callback/watcher lifetime but cannot recover that key. Native disconnect
failure and watcher panic currently share Result::Err despite different actual
native ownership/drain facts. No native execution was used to establish this.

Add Session::connect_retained with the same path/filter parameters, returning
Result<Connection<F>, ConnectFailure<F>>. ConnectFailure owns the original core
error and optional GENUINE Connection; into_parts transfers both values once.
Debug/Display expose only error/state, never Filter data, context/token/key or
paths. No Clone and no new public raw-key constructor/adoption. Pre-key path/open/
native failure has no fabricated connection. Immediately after successful
CfConnectSyncRoot, create the real Connection holding SAME native key/callbacks/
ContextToken/Arc<Filter>/active callback counter. Native-owner creation precedes
watcher spawn. Real watcher sender/join become explicit optional fields; an
unstarted watcher has neither, with no fabricated channel/thread. Watcher-spawn
failure returns the SAME actual Connection inside ConnectFailure, without hidden
rollback/drop. Caller owns its actual resources until explicit retained shutdown.
Legacy connect/connect_async remain compatible and keep their original rollback
error behavior; new retained callers exclusively use the retained API. All APIs
share actual native connect/disconnect ownership implementation where possible.

Add Connection::disconnect_retained returning RetainedShutdown: Complete,
CallbacksActive, WatcherStopping or DrainedWithWatcherFailure. Native CFAPI error
keeps original key/context/filter alive. Context removal/cancel occurs only after
actual successful native disconnect. Complete requires actual native disconnect,
zero active callbacks, and no running/unjoined watcher. A watcher never started
is separately observable and cannot qualify successful provider startup. Joined
watcher panic produces DrainedWithWatcherFailure only after actual native
shutdown/callback drain/failed join; it remains distinct from Complete on later
calls. Preserve failed watcher provenance, never manufacture normal success.
Legacy disconnect maps that result to its existing permanent error, preserving
ShutdownStatus variants and caller matches. Drop remains documented bounded
fallback; no timeout/count/receipt alone qualifies actual native shutdown.

Expose pure WatcherState metadata: NotStarted, Running, Exited (not yet joined),
Joined, FailedJoined. A successful startup caller requires Running, native
connected and its actual original owner/current proof checks. This metadata
never constructs/replaces custody. Keep retained errors/Connection owned on the
original caller worker; do not destroy ownership because IPC cancelled or replied.
No source callback/native effects under application Store locks. Do not add new
registry/framework, dependency, test hook, runtime adoption, raw-key fabrication,
unregister, data mutation, or destructive recovery. Preserve MIT attribution.

Current human phase: CODE ONLY. No tests/new test definitions/compiler/build/
dependency fetch/install/extraction/native execution/hosted CI dispatch. Existing
tests and native evidence remain unchanged and do not qualify this candidate.
Only standalone rustfmt syntax, source inspection and whitespace/signature review.
Source UNBUILT/type-unverified; native/runtime/E2E NOT_RUN. No release/publication.
After whole independent source review and eligible normal PR merge into fork main,
desktop uses ONLY that actual normal immutable commit in Cargo.toml and the same
single Cargo.lock git source, with package versions/edges unchanged. No cached
patch/unmerged-branch product pin or Cargo fetch/build is authorized by that step.

Owned source: w29_move_code, fork issue29/owned draft PR, exact
D:/projects/sambafied/_worktrees/fork-ok-nick-wincs/29-w29-connect-custody.
No disposable output created. Retain for unfinished bounded source until complete
reviewed normal landing/closure/clean pushed/no active locks/refs/process and
root minimum custody ACK; then helper-policy Git worktree/branch cleanup.
Primary157995578959a82642f90ff8d9b0b7fc9276d1f3 remains untouched, including its
existing target/qualification. Desktop W29 remains Partial until whole delivery.

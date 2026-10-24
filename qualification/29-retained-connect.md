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

Source implementation checkpoint: signed/pushed ced90b6 implements the complete
bounded API definitions in src/root/session.rs, connect.rs and mod.rs. Boxed
partial error keeps genuine ownership without a large by-value error; into_parts
moves that same Connection once. New native owner exists before watcher spawn;
optional fields correspond only to real watcher resources. Shared shutdown and
watcher metadata preserve the original failure distinction. Legacy connect uses
the common retained core followed by its rollback and bounded Drop fallback;
there is no promised deadline on native CFAPI itself. Source-only self review,
standalone rustfmt syntax and git whitespace pass; no compiler/type/runtime/
native/E2E results exist for this candidate. Root independent full source review
is pending; draft PR30 remains incomplete until findings/eligible normal gates.

No disposable output was created. Exact fork source retained by w29_move_code
for issue29/draft30 and desktop whole W29. Cleanup trigger remains independent
normal fork landing/closure/clean pushed/no active refs/locks/process and root
minimum custody ACK. Do not delete the primary's existing target/qualification.

Whole bounded issue29 source audit (candidate after ced90b6):

| ID | Required source behavior | Concrete source | Code audit | Runtime/E2E |
| --- | --- | --- | --- | --- |
| AC01 | Own successful native key/context before watcher creation | session.rs170 and Connection::new | Defined; coordinator source-reviewed | Not run |
| AC02 | Pre-key failure does not fabricate native owner | ConnectFailure::before_connect and connect_retained preflight | Defined; coordinator source-reviewed | Not run |
| AC03 | Partial watcher-start error transfers same real connection | session.rs188; owned error into_parts60 | Defined; coordinator source-reviewed | Not run |
| AC04 | Native shutdown error retains original context/key/callback ownership | disconnect_retained138; context removal only after native success149 | Defined; coordinator source-reviewed | Not run |
| AC05 | Optional fields describe actual watcher resources | attach_watcher114; NotStarted is explicit | Defined; coordinator source-reviewed | Not run |
| AC06 | Normal shutdown requires actual native disconnect/callback drain/watcher completion | retained common shutdown138-170 | Defined; coordinator source-reviewed | Not run |
| AC07 | Failed watcher join remains distinct from Complete | DrainedWithWatcherFailure plus legacy error mapping125 | Defined; coordinator source-reviewed | Not run |
| AC08 | Pure watcher metadata does not mint native authority | watcher_state173; five explicit states | Defined; coordinator source-reviewed | Not run |
| AC09 | Preserve legacy sync/async API and rollback/fallback contract | connect119, connect_async194, unchanged ShutdownStatus variants and bounded Drop | Defined; coordinator source-reviewed | Not run |
| AC10 | No Filter/key logging/adoption, dependency/test/MIT changes or unmerged product pin | manual error Debug/Display; private constructors; exact Git diff | Defined; coordinator source-reviewed | Not run |

Coordinator independently full-read production ced90b6 and actual base/callers,
verified GitHub signature/whitespace and reported no additional bounded finding.
Migration86e5f47 is included; this final audit is source evidence only. Existing
Cargo.toml/Cargo.lock/LICENSE/tests/fault injection source have zero diff from
actual base. No new tests/definitions/runtime/CI/native execution, no compiler/
type evidence. This candidate fixes the exact necessary API ownership contract,
not watcher overflow, all callback safety, all native qualification or whole
Sambafied desktop W29. Draft30 requires final exact-head coordinator eligibility
ACK before ready/normal merge. Product pin remains e7865a7 until actual normal
fork merge and immutable closure ACK; no branch/local-patch product integration.

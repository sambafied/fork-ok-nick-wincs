# Sambafied fork integration

This fork can build and evolve independently of upstream review. No upstream
merge, crates.io publication or Sambafied product adoption is required for local
builds. Work lands through signed, immediately pushed `sambafied/` branches and
PRs into this fork's `main`. Retain MIT notices. No release was published here.

## Imported source and decisions

- Original `ok-nick/wincs`: a5a5bfb4a5c2efe4af8663b61e9aaa27b0da4b00.
- Merged `ho-229/cloud-filter-rs`: fa771923f2ca35dab9116af576cb9af83f288678.
  Its 42 distinct commits and author history are retained. Reuses Result-returning
  callbacks, async bridge, rename/delete/progress fixes, owned placeholder handles,
  metadata/range helpers, implemented sync-root enumeration and information,
  attribute-state watcher, optional indexing, native sync/async behavior tests,
  documentation and Rust 1.90 token handling fixes.
- Reviewed `esdandreu/cloud-filter-rs` patch
  982d908fce4a659d734c2564d5b56d98940352f7. Ported its useful callback failure and
  disconnect logging approach. Did not import cosmetic unwrap-to-expect changes:
  those still panic and do not make error handling fallible.
- Other inspected forks offer error-framework churn, example-specific changes,
  CI-only changes, or no unique default-branch work. These are not imported as
  independent features. The donor already incorporates the useful token/CI fixes.

The two divergent original-upstream commits were reconciled by intent: preserve
the newer binding API and fallible ID parsing/tests, and accept signed icon
resource IDs through the new `SyncRootInfo::with_icon("path,-101")` string API.
`PlaceholderFile::create` does not retain the unused generic argument. Binding
version is `windows` 0.62.2 to match Sambafied; numbered operation/callback union
fields and WinRT/string APIs were migrated together. The package remains `wincs`
0.1.0, with `publish = false`; donor package/API names are not a compatibility
promise. No dependency was added to the product repository.

## API migration

`Connection::disconnect` reports native failure, `CallbacksActive`,
`WatcherStopping` or `Complete`. `CallbacksActive` is new: match it (or treat
every non-Complete status as "poll again"). Wait for Complete before
unregistering; Drop remains a fallback. `Connection::active_callbacks()` and
`is_connected()` expose shutdown state. Native disconnect does **not** drain
dispatched callbacks: fail or finish in-flight work and wait for
`active_callbacks() == 0` before disconnecting, or readers of orphaned callbacks
block for the platform's 60-second request timeout. See the
[issue 7 failure matrix](qualification/7-disconnect.md) and the
[AEM continuation prompt](HANDOVER.md).

The donor reorganizes APIs: `root::SyncRootInfo` plus `SyncRootId::register`
replaces `Registration`; metadata lives in `metadata`; handles and operations
live in `placeholder`; `filter` exports `Request`, synchronous `SyncFilter` and
async `Filter`. Fallible callbacks return `CResult` instead of reporting their
error through tickets alone. See the migrated examples; old top-level imports
must be updated. `SyncRootId::to_components` remains fallible. The legacy static
`FileExt::in_sync_root` explicitly returns E_NOTIMPL rather than panicking.

## Import hardening

- Async executor results use initialized Option state: an executor that does not
  complete its future returns NotSupported, never uninitialized memory.
- `CallbackContext` is an opaque integer token into a process-wide registry of
  Weak filter references, never a dereferenced pointer. The previous boxed-Weak
  pointer relied on disconnect ending callbacks, which the native suite
  disproved (issue 7). Removing a token is always safe; late callbacks resolve
  no filter and return. Resolved callbacks hold a counted guard.
- Watcher directory opens before provider connection. A NUL in the root path
  returns E_INVALIDARG instead of panicking. Thread-spawn failure rolls
  back the connection. Cancellation drains overlapped I/O before its buffer and
  OVERLAPPED can be destroyed. The notification buffer is DWORD-aligned.
- Connection drop does not panic on disconnect failure and limits its watcher
  join polling to five seconds. A delayed worker retains its own file/buffer and
  filter until exit. This is not a deadline on CfDisconnectSyncRoot itself or on
  arbitrary application callbacks.
- Native registration tests require explicit opt-in, create unique owned
  temporary roots/account IDs, refuse existing registrations, retain failed
  roots for diagnosis, and use filesystem APIs instead of shell command strings.
  They no longer adopt or recursively delete fixed C:\ test paths.

## Local validation

Use the Windows x64 MSVC toolchain and Windows SDK libraries:

```powershell
cargo fmt --all -- --check
cargo test --locked --lib --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo doc --locked --no-deps --all-features
```

Current native qualification on 11 October 2026: seven library and twelve native
cases PASS, zero ignored. Strict Clippy, format and docs PASS. Exact scope and
reproduction are in [issue 15/16 qualification](qualification/15-16-native.md).
Earlier issue13/14 failures and receipts remain historical evidence.
Cargo.lock is committed for repeatability; the package remains unpublished.

```powershell
$env:SAMBAFIED_CF_TEST_PARENT = '<absolute-existing-owned-fixture-parent>'
$env:SAMBAFIED_RUN_CLOUD_FILTER_TESTS = '1'
cargo test --locked --all-targets --all-features -- --test-threads=1
```

Without opt-in, behavior exits zero with NOT_RUN; that is not acceptance. Roots
use unique SID/account identities, unregister after disconnect or Rust unwind,
verify exact identity/path and absence, and retain bytes until owned cleanup.
Injected unwind is outside FFI, not proof of callback panic containment. Tests
use separate bounded client processes for population/hydration and write races.
Original exact entries/contents, empty and 128-entry/four-page population remain.

Native nonzero-USN dirty-to-clean remains unsupported on this host. It is still
characterized against direct CFAPI; original failing evidence is preserved. The
qualified alternative is `Placeholder::acknowledge_uploaded`: validate volume,
file identity and nonzero USN under an exclusive RH oplock/reference, then change
state before releasing the barrier. No zero-token/unprotected fallback. Bind the
observed `FileGeneration` to the exact verified immutable uploaded snapshot in
the engine. Existing writers/mappings defer acknowledgement; never hold over
network I/O. Raw `mark_in_sync` stays a low-level API, not the selected upload
acknowledgement protocol.

All Placeholder handle-taking methods normalize protected handles with referenced
Win32 lifetime. Guard clones share one native reference and retain the opaque
owner; failure acquisition is fallible and balanced. Ordinary File ownership
transfers once; protected asynchronous File conversion is rejected without
reverting the placeholder. Information clones own their header/identity. Update
metadata/range pointers and hydration offset/length are repaired. Request identity,
callback containment and other safety epic criteria remain open. No hosted CI
was dispatched and examples remain reference code requiring separate audit.

## Remaining product gates

This is a reviewed wrapper import, not a complete sync engine or production
qualification. Caller callbacks must not panic across native FFI. Further audit
must cover callback panic/error paths, metadata/range edge cases, watcher
overflow/recovery and reconnect, failure-retained context recovery, and remaining
panic-prone convenience APIs. Native hydration and directory population passed on the supplementary host;
cancellation, pin/free
space, logon/restart, signed package and Explorer menus/icons are still NOT_RUN
for this fork. Authentication, writes/uploads, conflicts and durable recovery
belong to the shared Sambafied engine.

Tracked by [Sambafied #188](https://github.com/sambafied/sambafied/issues/188),
with shared-engine prerequisites #186/#187. SMB remains available; WinFsp #194
remains the separate virtual-volume option. The owner requested no Explorer
restart, sign-out or package upgrade for this work.

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
- Callback context is one immutable boxed Weak owned by Connection. Callbacks
  borrow it, without taking raw Weak ownership repeatedly. Successful disconnect
  ends native callbacks before context release. A failed disconnect deliberately
  retains the small Weak context, avoiding a dangling pointer; it logs the error.
- Watcher directory opens before provider connection. Thread-spawn failure rolls
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

Native requalification on 11 October 2026: six library tests and strict Clippy
pass. Six native regression cases pass; one positive current-nonzero-USN gate is
explicitly ignored in that suite and FAILS when enabled. See
[issue 13/14 evidence](qualification/13-14-native.md). This is partial native
qualification, not production approval. Cargo.lock is committed for repeatability.

```powershell
# Existing owned directory, never a shared or production sync root:
$env:SAMBAFIED_CF_TEST_PARENT = '<absolute-owned-fixture-parent>'
$env:SAMBAFIED_RUN_CLOUD_FILTER_TESTS = '1'
cargo test --locked --all-targets --all-features -- --test-threads=1
# Separate production gate; currently FAILS (fork #15):
$env:SAMBAFIED_QUALIFY_CURRENT_USN = '1'
cargo test --locked --test behavior --all-features -- current_nonzero_usn_acceptance
```

The provider tests use a separate bounded client process to cause real population
and hydration; a provider enumerating its own root did not trigger callbacks.
The original three-entry/two-content assertions remain. Additional synchronous
and asynchronous cases exercise empty completion and 128 entries in four pages.
`FetchPlaceholders::pass_batch` supplies total count and explicit final-page state,
returns native EntriesProcessed, and leaves per-entry result checking to callers.
Only the final page disables on-demand population. Legacy pass remains one batch.

USN arguments now pass addresses of live mutable values. Conversion's USN is an
output, while mark-in-sync's USN is conditional input/output. Touched conversion,
state and info methods reference a real Win32 handle for protected CFAPI handles.
Remaining protected-handle/ownership paths require fork #16 audit. Native operation
results are read from the mutated parameter union only after successful execution;
request identity and other safety-epic requirements remain open.

Owned registration guards unregister after disconnect on success and Rust unwind,
verify exact identity/path and absence, and retain fixture bytes for diagnosis.
Injected unwind is outside FFI. They do not prove panic containment across native
callbacks. Remove only owned disposable fixtures after unregister checks. Examples
remain reference code requiring separate audit. Hosted CI was not dispatched.
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

# Issue 7: disconnect, in-flight callback lifetime and failure matrix

Qualified on 14 October 2026: Windows 10.0.26300 x64, NTFS `C:` (owned fixture
parent), rustc 1.98.1 MSVC, standard user. Supplementary host, not a supported
release row. The earlier 13 October success-path receipt (PR #19) is retained
below as history.

## Native finding: disconnect does not drain dispatched callbacks

`disconnect_during_inflight_callbacks` starts two separate-process reads of
cloud-only placeholders whose `fetch_data` callbacks each hold 1.5 s. With both
callbacks running, `CfDisconnectSyncRoot` returned after 64–86 µs in four runs
with both callbacks still executing. The prior wrapper freed its boxed callback
context immediately after that call, so a callback dispatched but not yet past
its context read could have read freed memory. PR #19's statement that
successful disconnect ends native callbacks is superseded.

A callback that finishes after disconnect can no longer complete its operation
(the connection key is dead). Its reader stayed blocked until the platform
request timeout: 60.1–60.8 s observed.

## Repair and contract

- `CallbackContext` is a non-zero integer token into a process-wide registry of
  `Weak` filter references; it is never dereferenced. The token is removed after
  successful native disconnect (and on connect rollback or failed Drop), after
  which callbacks resolve no filter. No context is leaked any more.
- Resolution increments a per-connection active counter under the registry read
  lock; removal takes the write lock. After removal, an observed zero is final.
- `ShutdownStatus::CallbacksActive` (new) is returned while callbacks that
  resolved before removal still execute filter code. `Complete` now means native
  disconnect succeeded, active callbacks are zero and the watcher joined.
  `Connection::active_callbacks()` and `is_connected()` expose state.
- Consumers must fail/finish in-flight work and wait for `active_callbacks() == 0`
  **before** `disconnect`, otherwise readers wait for the 60 s platform timeout.
- A failed native disconnect keeps the token registered; callbacks remain fully
  functional and disconnect can be retried. Watcher panic stays a sticky error.
- Root paths containing NUL return `E_INVALIDARG` instead of panicking.
- The callback table is copied by Windows during connect (it already moved out
  of `connect`'s frame before this change), so it needs no retention.
- No deadline is imposed on `CfDisconnectSyncRoot` or application callback code.
  A callback that never returns keeps the connection in `CallbacksActive`.

## Matrix

| Case | Test | Result |
|---|---|---|
| Disconnect during two concurrent slow recalls | `disconnect_during_inflight_callbacks` | PASS: `CallbacksActive` while running, `Complete` 1.50 s later only after both exits, active count drained, repeat close stays `Complete`, reconnect and close |
| Watcher spawn failure after native connect (injected) | `rollback_after_watcher_spawn_failure` | PASS: connect fails; control proves duplicate connect fails while connected; later connect succeeds, so rollback disconnected |
| Native disconnect failure x2 (injected before the native call) | `failed_disconnect_retains_callbacks` | PASS: error `ERROR_CLOUD_FILE_REQUEST_TIMEOUT`, still connected, separate-process recall reaches the filter (27–29 ms), retry reaches `Complete`, repeat stays `Complete` |
| Watcher panic (injected before any I/O) | `watcher_panic_is_sticky` | PASS: error on first and later close, native disconnect recorded, new connection succeeds |
| Token removal / late callback / guard lifetime | `filter::context` and `filter::proxy` unit tests | PASS |
| Repeat close / reconnect | `fallible_disconnect_repeat_reconnect` (existing) | PASS |

Fault plans use the test-only `fault-injection` feature, bound to one `Session`
(no global state). Totals: 11 library and 18 native tests, zero ignored, with
`--all-features --test-threads=1`. Format, strict Clippy with and without
features, and rustdoc with `-D warnings` pass. 55 owned fixture registrations
verified absent afterwards; fixture directories are retained for disposal.

```powershell
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --all-targets --all-features -- -D warnings
$env:RUSTDOCFLAGS = '-D warnings'; cargo doc --locked --no-deps --all-features
$env:SAMBAFIED_CF_TEST_PARENT = '<absolute-owned-fixture-parent-on-a-CldFlt-volume>'
$env:SAMBAFIED_RUN_CLOUD_FILTER_TESTS = '1'
cargo test --locked --all-targets --all-features -- --test-threads=1
```

The fixture parent must be on a volume where the Cloud Files filter is
attached; on this host `D:` returned `0x80070001` (see desktop-client-adobe #46).

## Residual gates

Real (non-injected) `CfDisconnectSyncRoot` failure and real thread-spawn
exhaustion are not producible here; injection exercises the same wrapper paths.
Callback panic containment (#4), exact request identity (#5), watcher overflow
(#8) and supported Windows rows (#10) remain separate. Not production
qualification and not an app-level result.

## History: 13 October success path (PR #19)

Explicit disconnect, idempotent repeat, reconnect and watcher-panic propagation
passed with seven library and thirteen native cases.

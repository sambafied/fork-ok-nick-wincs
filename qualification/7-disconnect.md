# Issue 7: observable disconnect

Validated again on 13 October 2026 on this Windows x64 MSVC workstation.

`Connection::disconnect(&mut self)` returns a native error while retaining
ownership for retry, `WatcherStopping` after native callbacks have ended but the
watcher is still draining, or `Complete` after a successful watcher join.
Repeated successful disconnect is idempotent. Watcher panic is a persistent
error. Unregister only after `Complete`; disconnect does not unregister or
delete files. Drop keeps the previous bounded fallback and failure-retained
Weak context. No deadline is promised for CfDisconnectSyncRoot or caller code.

The new `fallible_disconnect_repeat_reconnect` native test connects an owned
root, polls explicit disconnect to completion, repeats it, reconnects, completes
disconnect again, then unregisters and verifies absence. Existing assertions
remain unchanged. Seven library and thirteen native cases passed, zero ignored.
Format, strict all-target/all-feature Clippy and rustdoc passed.

Reproduce using an existing owned fixture parent and explicit native opt-in:

```powershell
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
$env:SAMBAFIED_CF_TEST_PARENT = '<absolute-owned-fixture-parent>'
$env:SAMBAFIED_RUN_CLOUD_FILTER_TESTS = '1'
cargo test --locked --all-targets --all-features -- --test-threads=1
cargo doc --locked --no-deps --all-features
```

Issue 7 remains open: native disconnect failure injection, slow in-flight
callback teardown, watcher panic injection, spawn failure and the full recovery
matrix are not qualified by the success-path reconnect test. Callback panic
containment and request identity remain separate safety gates. This is not
production qualification or an app-level CFAPI integration result.

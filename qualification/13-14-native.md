# Issues 13/14 native repair qualification — 11 October 2026

Scope: pointer safety, conversion output, native population and failure cleanup.
Supplementary host: Windows build 26300.9457 (26H2), NTFS, x64 MSVC. This does
not certify the advertised Windows 11 25H2 or 24H2 rows (fork #10).

Changes: real USN pointers; protected-handle normalization for conversion/state/
info; transparent placeholder-create array ABI; read native mutated operation
outputs; paged/empty population API with explicit final completion; external-client
native behavior tests and identity-checked registration guards. Metadata in test
placeholders includes identity, timestamps and in-sync state. No production root,
AEM tenant or package state was changed.

Results: six library tests pass, format and strict all-target Clippy pass; native
regression six pass/zero fail/one ignored. Original exact recursive entries and
file contents remain asserted; both provider types also pass 128-entry/four-page
and empty population. Injected Rust unwind releases registration and retains bytes.

The separate positive current nonzero USN gate FAILS with 0x80070179; a direct
windows-rs CfSetInSyncState control on the same real handle also rejects it.
Conversion writes zero on this host; that output is not a current concurrency
token. Stale-USN rejection and preservation of later bytes pass. An unconditional
state change establishes test setup only and is never approved as a production
clean-state substitute. Cause remains unresolved, tracked in
[issue 15](https://github.com/sambafied/fork-ok-nick-wincs/issues/15).

See retained [regression output](evidence/native-regression-2026-10-11.log) and
[positive gate failure](evidence/current-usn-acceptance-2026-10-11.log).
Reproduction is in FORK.md. Full safety epic #2 remains open, including other
protected-handle/ownership paths (#16), callback containment, request identity,
shutdown and supported-build/app/package qualification. Foundation/auth/read
work may proceed; production clean-state, eviction and automatic sync remain gated.

API contracts: [CfSetInSyncState](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfsetinsyncstate)
and [CfConvertToPlaceholder](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfconverttoplaceholder).

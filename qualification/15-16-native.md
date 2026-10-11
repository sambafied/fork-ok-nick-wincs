# Issues 15/16: generation acknowledgement and protected-handle audit

11 October 2026; Windows 26300.9457/26H2, NTFS, x64 MSVC. Supplementary host only;
supported 25H2/24H2/app/package coverage remains safety epic #2 and issue #10.

## Issue 15 decision and evidence preservation

Raw nonzero-USN dirty-to-clean CfSetInSyncState STILL rejects with 0x80070179;
direct windows-rs control agrees. CfUpdatePlaceholder also rejected the token.
The original failing receipts in qualification/evidence are unchanged. This is
not a claim that Windows was fixed. The acceptance alternative permitted by #15
is selected: explicit identity/USN validation under a short exclusive RH oplock
barrier, followed by native acknowledgement before that barrier is released.

`FileGeneration` contains volume identity, 128-bit file identity and nonzero NTFS
USN. `Placeholder::generation` observes a token; the engine must bind it to its
qualified immutable uploaded snapshot. `acknowledge_uploaded(path, expected)`
opens EXCLUSIVE|WRITE_ACCESS (no FOREGROUND), acquires a protected reference,
compares identity/USN and commits while retaining the reference. It rejects stale
and replaced generations. `UploadAcknowledgement::begin/commit` exposes the same
barrier for bounded local coordination; drop aborts without changing state.
Never retain a reference over network I/O. Existing writers/writable mapped
sections prevent the barrier; competing opens wait until its release.

The state syscall uses null only INSIDE this validated barrier. No raw/null-token
fallback exists in acknowledgement. The old raw positive gate is retired as the
production strategy, not relabeled a raw pass. Its failure remains characterized
in `usn_pointer_output_and_stale_generation`, and the new independently named
`exclusive_generation_acknowledgement` tests the selected contract. Errors are
returned; a post-state query failure is uncertain state, not eviction authority.
Zero conversion output is never used as an expected generation.

Blueprint and unchanged engine requirements are recorded in AEM
`docs/usn-acknowledgement-decision.md` under issue26 (AEM-005/006/007/009).
This primitive does not itself prove upload completion, snapshot coherence,
durable tracker recovery, or eligibility for production eviction/automatic sync.

## Issue 16 audit

| Surface | Correct ownership / handle behavior |
| --- | --- |
| convert, mark-in-sync, pin, update, info/fixed info, range info, hydrate | Ordinary handles pass directly; protected handles acquire a real Win32 reference across synchronous call. Native matrix passes both. |
| CfReference / CfGetWin32Handle | Failed reference returns E_HANDLE; invalid returned handle releases the acquired reference. |
| ArcWin32Handle clones | Share one successfully acquired native reference in a Rust Arc, retaining opaque owner. Last clone releases then permits CfCloseHandle; no ignored fallible native clone. |
| protected owner drop / break | Native reference can outlive Placeholder; queued conflicting opener proceeds after last guard releases. Subsequent acquisition fails safely. |
| std File transfer | Ordinary synchronous handle transfers once with ManuallyDrop; protected asynchronous conversion returns E_INVALIDARG without revert or unsafe asynchronous File wrapping. |
| PlaceholderInfo clone | Own copied header and exact identity bytes; no retained self-pointer, alignment or tail-offset ambiguity. |
| update pointer/range | Metadata borrow stays alive through syscall; nonempty range array passed correctly. |
| hydrate | Checked offset plus length, not mistaken end position; reversed/overflow bounds return E_INVALIDARG. |

## Native results

Seven library tests, twelve native cases, zero failures/ignored; strict Clippy,
format and docs PASS. [Final all-target output](evidence/15-16-all-targets-2026-10-11.log).
The original sync/async three-entry/two-content assertions plus empty/four-page
population remain. Added positive current generation, stale/replaced generation,
competing external writer, existing writer and surviving writable view refusal;
ordinary/protected method matrix; nonempty range invalidation with unaffected
native resident range; cloned info; 128 ownership cycles/handle counts; oplock
break and 128 failed reference acquisitions. Ordinary N+1/mapped bytes survive
unregister; fixture-generated range invalidation intentionally removes test data.

Diagnostic note: reading a dehydrated fixture with Full hydration and no remote
binary caused a 60s recall timeout, not an update hang. Final range test asserts
native resident ranges instead of attempting unsupported recall. Initial timed
out fixture process was stopped by exact owned PID and exact registration removed;
later diagnostic registrations verified absent. No other worker/package/Explorer
state changed. Disposable fixture bytes can be deleted after absence checks;
required small acceptance records remain in Git.

Remaining safety epic issues are separate; no crates.io release or production
application delivered. Both #15 and #16 are bounded dependency repairs, not
certification of the full wrapper or creative-app/remote engine behavior.

API references: [exclusive CFAPI open](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfopenfilewithoplock),
[reference balancing](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfreferenceprotectedhandle),
[grant conditions](https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/granting-oplocks),
[update pointers](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfupdateplaceholder).

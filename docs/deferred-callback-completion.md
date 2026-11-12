# Deferred callback completion

Sambafied desktop issue651 requires native callbacks to enqueue owned work and
return promptly. Copy all needed paths/scalars while `Request` and callback info
are valid. Neither type is permission to retain borrowed native pointers.

The original `FetchData` ticket may move into owned work. Its consuming
`fail_range` method reports failure for the original required byte range through
the existing TRANSFER_DATA command; invalid/empty/signed-overflow ranges refuse
before native execution. `Dehydrate::fail` similarly reports deferred refusal.
Retain the actual connection and callback-related owners until queued work and
completion attempts settle. A numeric `Request::transfer_key` is cancellation
matching data, not permission to manufacture another ticket.

All thirteen native entry points reject null arguments and contain Rust
unwinding before Request construction. Fallible callbacks use their own native
failure operation; notifications have no invented return status. Failure
reporting and final filter release are themselves contained. Invalid non-null
pointers and aborting panics cannot be recovered by Rust unwind handling.

Validation on Windows: 24 library tests and library Clippy with warnings denied
passed. These are library/source checks, not real deferred CFAPI I/O or desktop
shutdown acceptance. The desktop retains responsibility for admission bounds,
requested coverage, cancellation and actual task/registration settlement.

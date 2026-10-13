# Failed recall completion (#23)

Live AEM logout during the app integration revealed that a rejected fetch-data
callback left pending reads waiting. The wrapper submitted a zero-length failed
transfer, which overlaps no read. The fetch-data proxy now returns the callback's
required offset/length and a null ignored buffer on failure. The private
context-free Write::fail implementation is removed; this operation requires a
real range. Full operation-context propagation stays #5.

The added opted-in native regression creates a 131089-byte cloud-only file,
rejects the actual fetch-data callback with AuthenticationFailed, and requires a
separate process to receive an error within five seconds. It verifies the callback
ran and then explicitly disconnects/unregisters the owned fixture.

Reference: [CF_OPERATION_PARAMETERS transfer completion](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_operation_parameters).

13 October 2026, Windows 26300 x64 MSVC, NTFS: seven library and fourteen native
tests PASS, zero ignored. New separate-process failure returned in 26ms. Format
and strict all-target/all-feature Clippy PASS. Existing success, population, USN,
protected-handle and disconnect tests retain their assertions. This bounded
repair does not close the full request-context/panic/release safety epic.

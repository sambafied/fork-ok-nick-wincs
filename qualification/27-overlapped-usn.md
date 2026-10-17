# #27: asynchronous USN query completion and lifetime

17 October 2026. Requirement: a CFAPI-derived asynchronous Win32 handle must
receive a valid event-backed OVERLAPPED for FSCTL_READ_FILE_USN_DATA. The previous
null parameter violated the native contract even when warm calls happened to
complete. This does not establish the precise cause of the app's cold check-in
hang.

The request now owns stable input, aligned output, OVERLAPPED, byte count and
event storage. Immediate completion supports ordinary synchronous handles too.
ERROR_IO_PENDING is polled through GetOverlappedResult/event waits. A ten-second
logical deadline requests cancellation; Drop drains completion before releasing
buffers/event or returning to the caller that retains the protected reference.
A faulty driver can exceed that deadline during cancellation drain. This is not
a hard native-call deadline. USN record length, returned buffer bound, V2 format
and positive USN are checked before use.

Sources: [CfOpenFileWithOplock](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfopenfilewithoplock),
[DeviceIoControl](https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-deviceiocontrol),
[FSCTL_READ_FILE_USN_DATA](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_read_file_usn_data).

## Evidence

- Rust 1.98.1, Windows x64, serial tests, incremental disabled, dev codegen-units
  and build override codegen-units 1, dev debug 0: library tests PASS 12, zero
  ignored. Includes malformed/truncated/version/zero-USN parser rejection.
- New native metadata test PASS: 32 warm CFAPI foreground/protected observations
  plus cold FILE_READ_ATTRIBUTES, OPEN_REPARSE_POINT, OVERLAPPED observations.
  Cold attributes/length remain unchanged; zero hydration callbacks. It validates
  asynchronous USN metadata, not exclusive cold CFAPI data-open behavior.
- Diagnostic exclusive warm reopen failed safely with oplock-busy 0x80070322
  before the USN call. A foreground CFAPI cold data open caused hydration; it is
  not accepted as a metadata-only operation. The final cold test explicitly uses
  attributes-only Win32 access. Existing exclusive acknowledgement tests remain
  separate regression coverage.
- ERROR_IO_PENDING and cancellation/drain are implemented and reviewed in source;
  fast metadata operations may complete immediately. Forced pending/cancel branch
  execution is NOT_RUN; the metadata test does not claim it exercises that path.
- Initial compiler attempt crashed in windows-interface before compiling changed
  source (STATUS_ACCESS_VIOLATION). The bounded same-toolchain serial/debug0 pass
  above completed. No toolchain cycling or CI was used.
- Full native regression result is recorded below after execution.

Native fixtures belong only to the issue-27 unique C: test parent. Every exact
registration must be verified absent before deleting generated fixtures. Normal
client roots, the hung app root, normal package and AEM processes were untouched.
App pin update, app check-in acceptance and supported-platform qualification are
separate from this fork's bounded repair.

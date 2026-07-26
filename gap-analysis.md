# Gap analysis: rusty_win32 vs. the Win32 API surface (parity-loop, 2026-07-23)

Reference surface: the real Win32 API, as declared in mingw-w64's C headers
(`/usr/x86_64-w64-mingw32/include/`) — used as a local, reliable proxy for
`windows-sys`/`windows` (both are generated from the same official Win32
metadata Microsoft publishes, so function names/signatures match). A literal
`cargo public-api` diff wasn't used: `rusty_win32` wraps raw Win32 calls under
its own friendlier names rather than mirroring windows-sys's flat, 1:1-named
surface, so symbol-name diffing doesn't apply the way it does for
`rusty_libc` vs. the `libc` crate — this is a function-level coverage sweep
instead (same spirit, adapted mechanism).

Already wrapped (47 functions across `console`/`error`/`fs`/`handle`/`job`/
`path`/`pipe`/`process`/`time`/`volume`/`watch`) and already explicitly
out-of-scope per `ARCHITECTURE.md` (registry, ACLs/security descriptors,
service control, networking, ConPTY) are excluded below — see the sweep's
own notes for the full excluded list.

Platform column is omitted: every row is Windows-only (the whole crate is).
Breaking column is omitted: every row is a pure addition (no existing public
signature needs to change for any of these).

> **Status:** All candidate symbol groups identified across all categories (Strong candidates, Moderate candidates, Weak candidates, and the previously out-of-scope `registry`, `security`, `service`, `net`, and `conpty` subsystems) have been fully designed, implemented, tested, and merged as of PR #259.

## Strong candidates — concrete rush/rusty_lines consumer

| Symbol | Category | Reference | Est. size | Notes |
| --- | --- | --- | --- | --- |
| `GetCurrentDirectoryW`/`SetCurrentDirectoryW` | fn pair | `processenv.h` | S | The actual Win32 primitive behind `cd`/`pwd` — confirmed nothing in `src/` wraps CWD get/set at all today. |
| `FindFirstFileW`/`FindNextFileW`/`FindClose` | fn group | `fileapi.h` | L | Directory listing — `fs.rs` only stats individual paths today; no way to enumerate a directory's contents, needed by any future `ls`/tab-completion/glob that walks a directory. |
| `GetEnvironmentVariableW`/`SetEnvironmentVariableW` | fn pair | `processenv.h` | S each | Single-variable get/set — `environment_snapshot` (issue #19) only reads a full snapshot; `export`/`unset`/single-`$VAR` mutation needs live per-variable access. |
| `GetStdHandle`/`SetStdHandle` | fn pair | `processenv.h` | S each | `process.rs`'s own doc comment for `spawn_suspended` says redirection happens by "swapping the parent's std-handle slots before spawning" — this is that primitive, not currently owned by this crate. |
| `GetFullPathNameW` | fn | `fileapi.h` | M | Relative→absolute path canonicalization — `path.rs` has 8.3 short/long conversion but nothing for plain absolute-path resolution (`cd`/`pwd` internals). |
| `GetProcessId` | fn | `processthreadsapi.h` | S | HANDLE→pid, the reverse of already-wrapped pid→HANDLE (`open_by_pid`, issue #13) — needed anywhere rush holds a `HANDLE` and needs to report/print its pid. |
| `QueryFullProcessImageNameW` | fn | `processthreadsapi.h` | M | `list_processes`'s `ProcessEntry.exe_file` (issue #21) is a bare filename only — this is the gap for a `ps`-style listing that wants each process's full path. |
| `IsProcessInJob` | fn | `jobapi.h` | S | Windows nests every child a job member spawns into that same job automatically — checking this before `AssignProcessToJobObject` avoids surprise failures when rush is itself already inside an ambient job (common on CI runners/Windows Terminal). Directly relevant to the existing job-object process-group model (issues #14/#38). |
| `SetConsoleTitleW`/`GetConsoleTitleW` | fn pair | `wincon.h` | S each | Terminal title bar showing cwd/running command — the Windows analog of xterm's OSC title-setting escape sequence, a common shell feature. |
| `SetConsoleCursorPosition` | fn | `wincon.h` | S | `console.rs` only *reads* cursor position (`GetConsoleScreenBufferInfo`) — a raw-mode line editor (`rusty_lines`) doing multi-line prompt redraws needs to *move* the cursor too. |
| `FillConsoleOutputCharacterW`/`FillConsoleOutputAttribute` | fn pair | `wincon.h` | S each | Clear-to-end-of-line-style redraws — a common line-editor primitive for erasing stale characters after a shorter re-render. |
| `FlushConsoleInputBuffer` | fn | `wincon.h` | S | Discards pending input events — e.g. dropping stale keystrokes buffered during a slow command so they don't replay into the next prompt (common after Ctrl-C). |
| `GetNumberOfConsoleInputEvents` | fn | `wincon.h` | S | Non-blocking "how much is queued" check, the input-side parallel to already-wrapped `pipe_bytes_available` (`PeekNamedPipe`) for pipes. |
| `DisconnectNamedPipe` | fn | `namedpipeapi.h` | S | `pipe.rs`'s own gap: `create_server`/`connect_server` has no way to disconnect and reset a served pipe instance for reuse — currently single-use only. |
| `GetHandleInformation` | fn | `handleapi.h` | S | Read-side counterpart to already-wrapped `SetHandleInformation` (`handle::set_inheritable`) — verifying a handle's current inherit-flag state. |
| `GetDiskFreeSpaceExW` | fn | `fileapi.h` | S | `volume.rs` wraps volume metadata (`GetVolumeInformationW`) but not free-space — needed for a `df`-style builtin. |
| `Sleep` | fn | `synchapi.h` | S | Direct, trivial primitive behind a `sleep`/`usleep` builtin. |
| `GetSystemInfo` | fn | `sysinfoapi.h` | S | `dwNumberOfProcessors` is the primitive behind an `nproc`-equivalent builtin. |
| `GetComputerNameW`/`GetComputerNameExW` | fn | `sysinfoapi.h` | S | Hostname — `$HOSTNAME`, a shell prompt, or a `hostname` builtin. |
| `GlobalMemoryStatusEx` | fn | `sysinfoapi.h` | S | System memory totals/load — a `free`-style builtin or resource reporting. |
| `SetErrorMode` | fn | `errhandlingapi.h` | S | Without `SEM_FAILCRITICALERRORS`, a hardware/media error (e.g. an empty removable drive) pops a blocking GUI dialog that freezes a non-interactive shell/script run — real robustness gap for rush's startup path. |

## Moderate candidates — plausible builtins, not yet built in rush

| Symbol | Category | Reference | Est. size | Notes |
| --- | --- | --- | --- | --- |
| `CopyFileW`/`CopyFileExW` | fn | `winbase.h` | M | Backs a `cp` builtin. |
| `MoveFileW`/`MoveFileExW` | fn | `winbase.h` | M | Backs `mv`, cross-volume-safe rename. |
| `DeleteFileW` | fn | `winbase.h` | S | Backs `rm` (files). |
| `CreateDirectoryW`/`RemoveDirectoryW` | fn pair | `fileapi.h` | S each | Backs `mkdir`/`rmdir`. |
| `CreateHardLinkW` | fn | `winbase.h` | S | Hard-link creation, non-symbolic counterpart to already-wrapped `CreateSymbolicLinkW` — backs `ln` (no `-s`). |
| `GetTempPathW`/`GetTempFileNameW` | fn pair | `fileapi.h` | S each | Heredoc scratch files, or a `mktemp` builtin. |
| `GetPriorityClass`/`SetPriorityClass` | fn pair | `processthreadsapi.h` | S each (M pair) | Windows analog of `nice`/`renice` for background jobs. |
| `SuspendThread` + `Thread32First`/`Thread32Next` | fn group | `processthreadsapi.h`/`tlhelp32.h` | M | Closest Windows equivalent to `SIGSTOP`/`SIGCONT` (`bg`/`fg`/Ctrl-Z-style job stop) — Windows has no process-level stop primitive, needs per-thread suspend + enumeration. |
| `SetNamedPipeHandleState` | fn | `namedpipeapi.h` | M | Byte/message mode switch + blocking-behavior adjustment for named pipes — would let `pipe.rs` support non-blocking reads like `pipe_bytes_available` does for anonymous pipes. |
| `CreateMutexW`/`ReleaseMutex`/`OpenMutexW` | fn group | `synchapi.h` | M | Named cross-process mutex — Windows analog of `flock`, e.g. guarding concurrent writes to a shared history file from multiple rush instances. |
| `LockFileEx`/`UnlockFileEx` | fn pair | `fileapi.h` | M | Advisory file locking — alternative to the `CreateMutexW` option above for the same shared-history-file use case. |

## Weak/no clear consumer — round 2: filed anyway, per explicit direction

Round 1 (above) left these out because no `rush`/`rusty_lines` call site needs
them today. Per explicit user direction, the "needs a consumer" gate is
dropped for this round — filed for general API completeness instead, the
same framing already used for `volume`/`time` ("added for completeness, not
because any consumer currently wants it"). Two round-1 items stay excluded
on their own separate merits, not the consumer gate: `SetConsoleTextAttribute`
(superseded by the VT escape sequences this crate already supports) and
`ReadConsoleW`/`WriteConsoleW` (would contradict this crate's own documented
choice of `ReadFile` over `ReadConsole`) — adding either would give the crate
two contradictory ways to do the same thing. `SetLastError` also stays
excluded: the round-1 note already flagged it as "internal-hardening only,"
i.e. not a public-API candidate at all (it exists for a caller to force a
specific `GetLastError()` value for testing its own error paths, not a
feature this crate would expose).

| Symbol | Category | Reference | Est. size | Notes |
| --- | --- | --- | --- | --- |
| `GetProcessAffinityMask`/`SetProcessAffinityMask` | fn pair | `winbase.h` | S each | Read/set which CPUs a process may run on — the Windows analog of `taskset`/`sched_setaffinity`. |
| `GetExitCodeThread` | fn | `processthreadsapi.h` | S | Thread-level counterpart to already-wrapped `GetExitCodeProcess` — reads a terminated thread's exit code. |
| `GetThreadTimes` | fn | `processthreadsapi.h` | S | Thread-level counterpart to already-wrapped `GetProcessTimes` — per-thread CPU-time accounting. |
| `CreateSemaphoreW` | fn | `synchapi.h` | S | Counting semaphore, alongside already-wrapped mutex (`handle::create_mutex`) — the other standard Win32 synchronization primitive. |
| `WaitForSingleObjectEx`/`WaitForMultipleObjectsEx` | fn pair | `synchapi.h` | S each | Alertable-wait variants of the plain waits already used throughout this crate (`process::wait`, `console::wait_readable`) — adds `bAlertable`, relevant only alongside APCs, which this crate doesn't otherwise use. |
| `SleepEx` | fn | `synchapi.h` | S | Alertable-sleep variant of already-wrapped `Sleep`. |
| `SignalObjectAndWait` | fn | `synchapi.h` | S | Atomically signal one synchronization object and wait on another — avoids a signal/wait race a caller would otherwise have to accept. |
| `FindFirstVolumeW`/`FindNextVolumeW`/`FindVolumeClose` | fn group | `fileapi.h` | M | Enumerate volumes by GUID path, independent of drive-letter assignment — `volume.rs` currently only enumerates by drive letter (`logical_drives`). |
| `GetVolumePathNameW` | fn | `fileapi.h` | S | Maps an arbitrary path to the root path of the volume it's on — the reverse direction of `volume.rs`'s existing per-volume calls. |
| `GetCompressedFileSizeW` | fn | `fileapi.h` | S | On-disk (compressed) size vs. `fs::stat`'s logical size — meaningful only for NTFS-compressed files, a real but narrow gap. |
| `CompareObjectHandles` | fn | `handleapi.h` | S | Ask Windows directly whether two handle values refer to the same kernel object — the documented-correct alternative to comparing raw handle values, which isn't guaranteed reliable. |
| `GetNamedPipeInfo` | fn | `namedpipeapi.h` | S | Reads back a named pipe's own type/mode/buffer-size configuration — the read-side counterpart to `pipe::create_server`'s creation-time parameters. |
| `TransactNamedPipe`/`CallNamedPipeW` | fn pair | `namedpipeapi.h` | M | One-shot message-mode pipe transactions (write-then-read in a single call) — an alternative to `pipe.rs`'s existing separate connect/read/write calls for simple request-response protocols. |
| `OpenJobObjectW` | fn | `jobapi2.h` | S | Open a named Job Object by name — `job::create` only creates anonymous jobs today; this is the reverse direction, matching `pipe::open_client`'s "open by name" shape. |
| `GetTickCount64` | fn | `sysinfoapi.h` | S | Milliseconds-since-boot monotonic counter — `time.rs` already wraps `QueryPerformanceCounter` for high-resolution monotonic time, so this is a coarser, simpler alternative some callers may still prefer for its trivial units. |
| `GetSystemDirectoryW`/`GetWindowsDirectoryW` | fn pair | `sysinfoapi.h` | S each | The `C:\Windows\System32`/`C:\Windows` paths — standard well-known-location primitives, the Windows analog of resolving `/usr/bin`. |
| `GetLogicalProcessorInformation` | fn | `sysinfoapi.h` | M | Detailed CPU topology (cores/logical processors/cache layout) beyond `process::logical_processor_count`'s single number — variable-length output array, needs the same growing-buffer pattern already used elsewhere. |
| `AddVectoredExceptionHandler`/`SetUnhandledExceptionFilter` | fn pair | `errhandlingapi.h` | M | Structured-exception-handling hooks — the closest Windows analog to installing a Unix `SIGSEGV`/`SIGABRT` handler; a real gap for anything wanting to catch/report a crash before the OS's default unhandled-exception UI takes over. |
| `GetConsoleProcessList` | fn | `wincon.h` | S | Lists the pids attached to the current console — e.g. "is anything else still attached to this console" checks. |
| `GetConsoleWindow` | fn | `wincon.h` | S | The `HWND` of the console window, if any (a headless/service process has none) — needed by anything that wants to manipulate the console window itself (position/focus) via ordinary `user32` window APIs, out of this crate's scope beyond returning the handle. |
| `AllocConsole`/`FreeConsole`/`AttachConsole` | fn group | `wincon.h` | M | Attach to, detach from, or allocate a brand-new console for the calling process — needed by a GUI-subsystem process that wants to acquire a console on demand (this crate's own tests already use `AllocConsole` internally for CI, per `console.rs`'s test helpers, but don't expose it as public API). |
| `GetLargestConsoleWindowSize`/`SetConsoleScreenBufferSize`/`SetConsoleWindowInfo` | fn group | `wincon.h` | M | Console screen-buffer/window-size configuration beyond the read-only `window_size` this crate already has — the write side of console geometry. |

## Round 2: previously out-of-scope subsystems — added per explicit direction

Round 1 excluded these per `ARCHITECTURE.md`'s own documented non-goals
(registry, ACLs/security descriptors, service control, networking, ConPTY) —
an architectural-boundary call, not a "no consumer" one. Per explicit user
direction, these boundaries are being crossed this round; `ARCHITECTURE.md`
is updated separately to reflect the new scope. Each subsystem below got its
own dedicated research pass (mingw-w64 header reading, in two cases a
compiled C probe) rather than a cursory listing, the same rigor round 1's
strong/moderate candidates got.

### `registry` (new module) — `winreg.h`

| Symbol | Category | Reference | Est. size | Notes |
| --- | --- | --- | --- | --- |
| `HKEY_CLASSES_ROOT`/`HKEY_CURRENT_USER`/`HKEY_LOCAL_MACHINE`/`HKEY_USERS`/`HKEY_CURRENT_CONFIG` | const group | `winreg.h` | S | Sentinel `HKEY` root values every other call starts from — `pub const`s, the same raw-sentinel convention `handle.rs` uses for `STD_INPUT_HANDLE`. |
| `RegOpenKeyExW`/`RegCloseKey` | fn pair | `winreg.h` | S | Open a subkey / close the handle — the registry analog of `handle::close`'s "must close what you open," but via `RegCloseKey`, since an `HKEY` is a distinct handle kind from `HANDLE`. |
| `RegCreateKeyExW` | fn | `winreg.h` | S | Open-or-create in one call, reporting via `lpdwDisposition` which one happened — idempotent "ensure this key exists" setup. |
| `RegQueryValueExW` | fn | `winreg.h` | M | Core value-read primitive — growing-buffer retry (`ERROR_MORE_DATA`), same idiom as `path::search_path`/`fs::final_path`, dispatching on `dwType` (`REG_SZ`/`REG_EXPAND_SZ`/`REG_DWORD`/`REG_QWORD`/`REG_BINARY`/`REG_MULTI_SZ`/`REG_NONE`) into a `RegistryValue` Rust enum rather than a raw byte blob. |
| `RegSetValueExW` | fn | `winreg.h` | M | Write-side counterpart — encodes a `RegistryValue` back into the `dwType`+byte-buffer shape each `REG_*` type expects (UTF-16 NUL-termination for `REG_SZ`, double-NUL for `REG_MULTI_SZ`). |
| `RegDeleteValueW` | fn | `winreg.h` | S | Remove one named value under an open key. |
| `RegDeleteKeyExW` | fn | `winreg.h` | S | Remove a leaf subkey (must have no subkeys of its own); takes a `REGSAM` (`KEY_WOW64_64KEY`/`KEY_WOW64_32KEY`) to target the right registry view reliably on WOW64. |
| `RegEnumValueW` | fn group | `winreg.h` | L | Enumerate every value under a key by index until `ERROR_NO_MORE_ITEMS` — the value-side analog of `fs::read_dir`'s iterator shape, but each item's name/data buffers grow independently, plus the same type-dispatch as `RegQueryValueExW` per item. |
| `RegEnumKeyExW` | fn group | `winreg.h` | M | Enumerate a key's immediate subkey names by index, plus each one's last-write time via `PFILETIME` — reuses this crate's already-duplicated-per-module `FILETIME` mirror (`fs.rs`/`time.rs`/`job.rs`/`console.rs` each have one). |
| `RegQueryInfoKeyW` | fn | `winreg.h` | M | Subkey/value counts and max name/data lengths in one call — the "ask how big first" pattern `fs::final_path` already uses, needed to pre-size buffers before enumerating. |
| `RegFlushKey` | fn | `winreg.h` | S | Forces a key's changes to disk immediately instead of Windows' lazy flush — the registry analog of `FlushFileBuffers`, a real durability gap for settings writes right before a risky operation. |
| `RegDeleteTreeW` | fn | `winreg.h` | S | Recursively deletes a subkey and everything under it in one call — without it, `RegDeleteKeyExW`'s leaf-only restriction forces a hand-rolled enumerate-and-recurse loop. |

Design notes: `HKEY` gets its own `pub type HKey = *mut core::ffi::c_void` rather than reusing `handle::RawHandle` (closed via `RegCloseKey`, not `CloseHandle`); `REGSAM` access masks (`KEY_READ`/`KEY_WRITE`/`KEY_ALL_ACCESS`/`KEY_WOW64_64KEY`) exposed as raw policy-free consts, matching `FILE_ATTRIBUTE_*`/`MOVEFILE_*` elsewhere. Explicitly excluded: transacted-registry variants, `RegConnectRegistryW` (remote registry), WOW64-reflection functions (deprecated since Windows 7), hive load/unload (`RegLoadKey`/`RegSaveKey`/etc., needs `SeRestorePrivilege`), `RegGetKeySecurity`/`RegSetKeySecurity` (ACLs — its own module below), `RegNotifyChangeKeyValue` (a watch/notify primitive, better scoped as a `watch.rs` follow-up than folded into CRUD).

### `security` (new module) — `aclapi.h`/`securitybaseapi.h`

Scope: file/directory owner and DACL inspection+modification (an
`icacls`/`ls -l`/`chmod`/`chown`-equivalent), not a from-scratch
reimplementation of the whole Windows security model.

| Symbol | Category | Reference | Est. size | Notes |
| --- | --- | --- | --- | --- |
| `GetNamedSecurityInfoW`/`SetNamedSecurityInfoW` (+ `LocalFree`) | fn pair | `aclapi.h` | L | Core round-trip: path → owner `PSID`/DACL `PACL` pointers, and back — the modern, path-based entry point an `icacls`/`ls -l`/`chmod`/`chown`-equivalent is built on. |
| `GetAclInformation` + `GetAce` | fn group | `securitybaseapi.h` | L | Enumerate a DACL's ACEs one at a time — turns an opaque DACL into the human-readable permission list `icacls`/`ls -l` displays. |
| `SetEntriesInAclW` | fn | `aclapi.h` | L | Builds a new ACL from an existing one plus add/replace/remove `EXPLICIT_ACCESS_W` entries — the primitive behind `icacls /grant`/`/deny`. |
| `BuildTrusteeWithSidW`/`BuildTrusteeWithNameW` | fn pair | `aclapi.h` | S | Wraps a SID (or raw name) into the `TRUSTEE_W` shape `SetEntriesInAclW`'s entries require. |
| `LookupAccountSidW`/`LookupAccountNameW` | fn pair | `winbase.h` | M | SID↔name resolution — turns an owner/ACE SID into a `"DOMAIN\name"` string for display, and a `"user"`/`"group"` string into the SID `chown`/`SetEntriesInAclW` need. |
| `ConvertSidToStringSidW`/`ConvertStringSidToSidW` | fn pair | `sddl.h` | M | `S-1-5-...` string form — the fallback `icacls` itself uses when a SID can't be resolved to a name (orphaned/foreign/deleted account). |
| `InitializeAcl` + `AddAccessAllowedAce`/`AddAccessDeniedAce` | fn group | `securitybaseapi.h` | M | Lower-level build-a-DACL-from-scratch alternative to `SetEntriesInAclW` — useful for a brand-new object's initial ACL. |
| `GetLengthSid`/`IsValidSid`/`CopySid` | fn group | `securitybaseapi.h`/`winnt.h` | S | Safe handling of an opaque, variable-length `PSID` blob — sizing and validity, needed anywhere a SID must be copied out of a short-lived buffer into this crate's own storage. |
| `EqualSid` | fn | `securitybaseapi.h` | S | Byte-correct SID comparison (naive memory comparison isn't safe — a `PSID`'s trailing sub-authority count varies). |
| `CreateWellKnownSid` | fn | `securitybaseapi.h` | S | Constructs well-known SIDs (Everyone, Administrators, SYSTEM) without a name-lookup round trip. |
| `ConvertSecurityDescriptorToStringSecurityDescriptorW`/`ConvertStringSecurityDescriptorToSecurityDescriptorW` | fn pair | `sddl.h` | M | Basic SDDL whole-descriptor convert helpers — a debug/snapshot ("`icacls /save`"-style) string representation of a file's full permission state. |

Design notes: `SID` and the self-relative security-descriptor blob are
Windows' famously variable-length, self-describing structures — treated as
opaque byte buffers manipulated only through the accessor functions above,
never as a locally-defined fixed-layout struct (unlike this crate's usual
`_Static_assert`-verified mirrors). `TRUSTEE_W`/`EXPLICIT_ACCESS_W` are
genuinely fixed-size and get an ordinary FFI-mirror struct with layout
verification; `ACL`/`ACE_HEADER`/`ACCESS_ALLOWED_ACE` get a fixed-*header*-only
struct in the same style `fs.rs` already uses for
`ReparseDataBufferSymlinkHeader`. Explicitly excluded: SACLs/auditing,
privilege/token manipulation (a separate subsystem — `AdjustTokenPrivileges`
et al.), impersonation, integrity levels/mandatory labels, capability
SIDs/AppContainer, claims-based security, tree-wide/inheritance-source
variants, `AccessCheck`-based runtime permission evaluation, and the
low-level absolute-SD plumbing (`MakeAbsoluteSD`/`MakeSelfRelativeSD`)
unneeded once `GetNamedSecurityInfoW`'s pre-split pointers are the entry
point.

### `service` (new module) — `winsvc.h`

Scope: a `systemctl`-equivalent (list, query, start, stop a named service),
not service installation/configuration tooling or writing a service host
process.

| Symbol | Category | Reference | Est. size | Notes |
| --- | --- | --- | --- | --- |
| `OpenSCManagerW`/`OpenServiceW`/`CloseServiceHandle` | fn group | `winsvc.h` | S | Connect to the local SCM and open a handle to one named service — the `desired_access`-bitmask-parameter pattern already used by `process::open_by_pid`/`open_thread`. |
| `EnumServicesStatusExW` | fn | `winsvc.h` | L | List every service with current status — the core of a `systemctl list-units`-equivalent; growable buffer, resume handle for paging, fixed-size `ENUM_SERVICE_STATUS_PROCESSW` records whose name fields are pointers into the same buffer. |
| `QueryServiceStatusEx` (`SC_STATUS_PROCESS_INFO`) | fn | `winsvc.h` | M | One named service's live status including backing pid — supersedes the older pid-less `QueryServiceStatus`. |
| `StartServiceW` | fn | `winsvc.h` | S | Start an already-installed service (zero-argument case only — `lpServiceArgVectors` only matters for driver-style services, out of scope). |
| `ControlService` | fn | `winsvc.h` | S | Stop/pause/continue/interrogate via one `dwControl`-selected call; caller polls `QueryServiceStatusEx` afterward to see state settle, the same poll-don't-block shape `job::process_ids` already uses. |
| `QueryServiceConfigW` | fn | `winsvc.h` | M | Static config (start type, binary path, display name) for a `systemctl show`-style detail view — same pointers-into-buffer shape as the enumeration struct above. |
| `GetServiceDisplayNameW`/`GetServiceKeyNameW` | fn pair | `winsvc.h` | S each | Translate between a service's short key name and human-readable display name. |
| `EnumDependentServicesW` | fn | `winsvc.h` | M | List services depending on a given one — the "will stopping this break something else" check before calling `ControlService`. |

Design notes: `SERVICE_STATUS`/`SERVICE_STATUS_PROCESS` are flat, fixed-size,
pointer-free structs (ordinary `_Static_assert`-probe candidates, no
different from `job.rs`'s existing limit-info structs); `ENUM_SERVICE_STATUS_PROCESSW`/`QUERY_SERVICE_CONFIGW`
are also fixed-size *records* but their string fields are pointers into a
separately-packed variable-length string region within the same allocation
— needs a "chase a pointer to a NUL-terminated wide string not necessarily
2-byte-aligned relative to the buffer start" reader, distinct from
`fs.rs`'s embedded-fixed-array approach for `WIN32_FIND_DATAW`. Neither
opening the SCM nor querying status inherently needs administrator rights;
starting/stopping a specific service commonly does, per that service's own
DACL — an ordinary `ERROR_ACCESS_DENIED` `Win32Error`, not something this
crate hardcodes or works around. Explicitly excluded: service
installation/reconfiguration (`CreateServiceW`/`ChangeServiceConfigW`/
`DeleteService`), writing a service host process itself
(`StartServiceCtrlDispatcherW`/`RegisterServiceCtrlHandlerW`/`SetServiceStatus`),
service ACLs (`QueryServiceObjectSecurity`/`SetServiceObjectSecurity` — the
`security` module's territory), SCM-wide locking, push-notification
callbacks (`NotifyServiceStatusChangeW` — callback-based, a poor fit for
this crate's synchronous style; polling `QueryServiceStatusEx` is the
realistic answer), and `QueryServiceConfig2W`'s many info-level variants.

### `net` (new module) — `winsock2.h`/`ws2tcpip.h`

Scope: basic TCP/UDP client+server socket programming — the same core
subset `rusty_libc` (this crate's Unix-side sibling) wraps for POSIX
sockets. Explicitly excluded: overlapped/IOCP async socket I/O
(`WSARecv`/`WSASend`/`AcceptEx`/`ConnectEx`), raw sockets,
multicast/broadcast beyond basic `setsockopt`, `WSAIoctl`, `select`/`WSAPoll`
(a single-socket "wait until readable" wrapper is a plausible small
follow-up, parallel to `console::wait_readable`, via `WSAEventSelect` — not
`select()` itself), IPv6 extensions beyond what `getaddrinfo` already
handles transparently, and legacy 16-bit-era compatibility surface.

| Symbol | Category | Reference | Est. size | Notes |
| --- | --- | --- | --- | --- |
| `WSAStartup`/`WSACleanup` | fn pair | `winsock2.h` | M | The one primitive with no POSIX/`rusty_libc` analog — every other Winsock call is documented UB before a matching `WSAStartup`/after `WSACleanup`; needs a verified `WSADATA` layout. Windows reference-counts nested calls internally, so no shared guard/RAII type is needed — two plain functions, ordering documented in both doc comments, matching this crate's existing no-`Drop`-anywhere convention. |
| `socket`/`closesocket` | fn pair | `winsock2.h` | M | Socket lifecycle create/destroy — introduces `pub type RawSocket = usize` (matching `std::os::windows::io::RawSocket` and mingw's `SOCKET` typedef), not `handle::RawHandle` — a `SOCKET` is a distinct handle namespace closed via `closesocket`, never `CloseHandle`. |
| `bind` | fn | `winsock2.h` | S | Attach a local address/port to a socket. |
| `listen` | fn | `winsock2.h` | S | Mark a bound TCP socket passive/listening. |
| `accept` | fn | `winsock2.h` | M | Accept one incoming TCP connection, returning a new socket plus the peer's address. |
| `connect` | fn | `winsock2.h` | S | TCP client connect (or fix a UDP socket's default peer). |
| `send`/`recv` | fn pair | `winsock2.h` | S | Byte-slice I/O on a connected socket — thin wrappers matching `console::read`/`console::write`'s shape. |
| `sendto`/`recvfrom` | fn pair | `winsock2.h` | M | Connectionless datagram I/O — the bare UDP round trip, each call marshaling a `sockaddr_in`(6) address. |
| `shutdown` | fn | `winsock2.h` | S | Half-close a TCP socket's send/receive direction before `closesocket`, for a clean FIN. |
| `setsockopt`/`getsockopt` | fn pair | `winsock2.h` | M | Generic option get/set, plus `SO_REUSEADDR`/`SO_RCVTIMEO`/`SO_SNDTIMEO`/`TCP_NODELAY`/`SO_ERROR`. |
| `getsockname`/`getpeername` | fn pair | `winsock2.h` | S each | Read back the local bound address (e.g. after binding port `0`) and the peer address of a connected socket. |
| `getaddrinfo`/`freeaddrinfo` | fn group | `ws2tcpip.h` | L | Hostname/service-name → address resolution — needs a verified `addrinfo` layout, hints construction, and walking Windows' own linked list before freeing it. |
| `sockaddr_in`/`sockaddr_in6` construct + parse | struct + fn group | `ws2def.h`/`ws2ipdef.h` | L | Two verified struct layouts plus small `{ip, port}`↔struct constructor/accessor functions — every address-taking row above needs one. |
| `htons`/`htonl`/`ntohs`/`ntohl` | fn group | `winsock2.h` | S | Byte-order conversion — ports/raw IPv4 fields are always network byte order. |

Design notes: `timeval` is *not* needed (Windows' `SO_RCVTIMEO`/`SO_SNDTIMEO`
take a plain millisecond `DWORD`, unlike POSIX); the four structs needing
verified layouts are `sockaddr_in` (16 bytes), `sockaddr_in6` (28 bytes),
`addrinfo` (48 bytes, 8-aligned), and `WSADATA` (~408 bytes with padding) —
confirm all four with this crate's usual compiled probe before implementing,
the hand-derived numbers above are a starting point only.

### `conpty` (new module) — `wincon.h`/`processthreadsapi.h`/`winbase.h`

The primitive a terminal-emulator-hosting shell (or anything wanting to
spawn a fully-interactive child, not just redirected stdio) needs — real
value, previously excluded for an architectural-boundary reason
(`rusty_lines` reads its own inherited stdin rather than hosting a child's
terminal), not a "no consumer" one.

| Symbol | Category | Reference | Est. size | Notes |
| --- | --- | --- | --- | --- |
| `CreatePseudoConsole`/`ResizePseudoConsole`/`ClosePseudoConsole` | fn group | `wincon.h` | M | The ConPTY lifecycle — create a pseudoconsole bound to a pipe pair, live-resize on terminal-size-change, tear down. Declared in this machine's mingw-w64 headers but gated behind `NTDDI_VERSION >= NTDDI_WIN10_RS5` (Windows 10 1809) — a compiled verification probe needs `-D_WIN32_WINNT=0x0A00 -DWINVER=0x0A00 -DNTDDI_VERSION=0x0A000006` to see past the guard; this machine's `libkernel32.a` import library already has stub symbols for all three, so linking is not blocked. |
| `InitializeProcThreadAttributeList`/`UpdateProcThreadAttribute`/`DeleteProcThreadAttributeList` | fn group | `processthreadsapi.h` | M | The generic (pre-ConPTY, Vista-era) extended-process-attribute mechanism — the only way to hand an `HPCON` to `CreateProcessW`. Query-then-allocate opaque-byte-buffer pattern (`PROC_THREAD_ATTRIBUTE_LIST`'s true size is knowable only at runtime, from the first call's own size-out) — new territory for this crate, distinct from its existing "retry a UTF-16 string buffer at the size the API reports" idiom. |
| New `spawn_suspended_with_pseudoconsole` fn + `STARTUPINFOEXW` struct + `EXTENDED_STARTUPINFO_PRESENT` | fn + struct + const | `winbase.h`/`processthreadsapi.h` | L | A **wholly new function** alongside `process::spawn_suspended` (verified: does not require changing that function's existing public signature) that builds a `StartupInfoExW` (verified via compiled probe: 112 bytes, 8-byte align, `lpAttributeList` at offset 104 — exactly the existing, already-verified `StartupInfoW` plus one trailing pointer) instead of a bare `StartupInfoW`, sets `EXTENDED_STARTUPINFO_PRESENT`, and drops the `inherit_handles` parameter (irrelevant once a pseudoconsole supplies the child's console I/O). |
| `Hpcon` type alias + `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` | type alias + const | `wincon.h`/`winbase.h` | S | `HPCON` is an opaque 8-byte handle-shaped pointer — its own type (`type Hpcon = *mut c_void`), not `RawHandle`, since its only valid destructor is `ClosePseudoConsole`, never `CloseHandle`. |

Design notes: end-to-end call sequence is two anonymous pipe pairs
(`handle::create_pipe`) → `CreatePseudoConsole` bound to one pipe's read end
and the other's write end (closing the caller's own copies of those two
ends right after, per Microsoft's guidance) → build the attribute list via
the two-call query-then-allocate protocol → `UpdateProcThreadAttribute`
binds the `HPCON` in → `CreateProcessW` with `EXTENDED_STARTUPINFO_PRESENT`
and a pointer to the embedded `StartupInfoW` field (not the whole
`StartupInfoExW`) → `ResizePseudoConsole` any time afterward →
`DeleteProcThreadAttributeList` + free the buffer, then (after the child
exits) `ClosePseudoConsole`.

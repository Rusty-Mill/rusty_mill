//! Permitted raw Win32 surface.

pub use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, RtlNtStatusToDosError, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS,
    ERROR_DIRECTORY, ERROR_DIR_NOT_EMPTY, ERROR_FILE_EXISTS, ERROR_FILE_NOT_FOUND,
    ERROR_INVALID_PARAMETER, ERROR_NOT_A_REPARSE_POINT, ERROR_NO_MORE_FILES, ERROR_PATH_NOT_FOUND,
    ERROR_SHARING_VIOLATION, HANDLE, INVALID_HANDLE_VALUE, NTSTATUS, STATUS_ACCESS_DENIED,
    STATUS_DELETE_PENDING, STATUS_DIRECTORY_NOT_EMPTY, STATUS_FILE_IS_A_DIRECTORY,
    STATUS_NOT_A_DIRECTORY, STATUS_OBJECT_NAME_COLLISION, STATUS_OBJECT_NAME_INVALID,
    STATUS_OBJECT_NAME_NOT_FOUND, STATUS_OBJECT_PATH_NOT_FOUND, STATUS_SHARING_VIOLATION,
    STATUS_SUCCESS, UNICODE_STRING,
};
// `Spawner::is_alive`: `STILL_ACTIVE` is `GetExitCodeProcess`'s
// documented sentinel for "hasn't exited yet" — a `Foundation` constant
// (typed `NTSTATUS`/`i32`, not `Threading`), despite being consumed
// exclusively through the `Threading` process APIs above.
pub use windows_sys::Win32::Foundation::STILL_ACTIVE;
// R2-equivalent containment (Rusty-Mill fs slice): `NtCreateFile`'s
// documented failure when `OBJ_DONT_REPARSE` (`nt_surface`'s own
// admission) rejects a reparse point encountered during resolution —
// the NT counterpart of Linux `openat2`'s `ELOOP`/`ErrorKind::
// FilesystemLoop`, mapped the same way in `sys::errmap::kind_of_ntstatus`.
pub use windows_sys::Win32::Foundation::STATUS_REPARSE_POINT_ENCOUNTERED;
pub use windows_sys::Win32::Foundation::{
    DuplicateHandle, SetHandleInformation, DUPLICATE_SAME_ACCESS, ERROR_BROKEN_PIPE,
    HANDLE_FLAG_INHERIT, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
pub use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
pub use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FileBasicInfo, FileDispositionInfo, FileFullDirectoryInfo, FlushFileBuffers,
    GetFileInformationByHandle, GetFileInformationByHandleEx, ReadFile, SetFileInformationByHandle,
    WriteFile, BY_HANDLE_FILE_INFORMATION, DELETE, FILE_APPEND_DATA, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO, FILE_DISPOSITION_INFO,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FULL_DIR_INFO, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
    FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FILE_TRAVERSE, FILE_WRITE_DATA, MAXIMUM_REPARSE_DATA_BUFFER_SIZE,
    OPEN_EXISTING, SYNCHRONIZE,
};
// test -ef's donor material (D11, faccessat slice's sibling):
// GetFileInformationByHandle's legacy 32-bit volume-serial +
// 64-bit file-index pair is the same same-file identity
// std::os::windows::fs::MetadataExt::file_index historically exposed —
// no new windows-sys feature needed, already Win32_Storage_FileSystem.
// Symlink slice: DeviceIoControl is Win32_System_IO (already enabled below
// for IO_STATUS_BLOCK); FSCTL_{SET,GET}_REPARSE_POINT need the separate
// Win32_System_Ioctl feature, and IO_REPARSE_TAG_SYMLINK needs
// Win32_System_SystemServices — both added to the workspace Cargo.toml
// for this slice.
pub use windows_sys::Win32::System::Console::{
    GetStdHandle, SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_C_EVENT,
    STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
pub use windows_sys::Win32::System::Ioctl::{FSCTL_GET_REPARSE_POINT, FSCTL_SET_REPARSE_POINT};
pub use windows_sys::Win32::System::SystemServices::IO_REPARSE_TAG_SYMLINK;
pub use windows_sys::Win32::System::IO::DeviceIoControl;
// The terminal cluster (extraction map D9, via the rusty_win32 donor):
// mode get/set doubles as the isatty probe; the screen-buffer query's
// srWindow is the viewport (the size a tty reports), not the scrollback
// buffer; the VT bits make a Win10+ console speak the same raw-mode
// dialect as a Unix tty.
pub use windows_sys::Win32::System::Console::{
    GetConsoleMode, GetConsoleScreenBufferInfo, SetConsoleMode, CONSOLE_MODE,
    CONSOLE_SCREEN_BUFFER_INFO, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT,
    ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
};
// Console *acquisition* (D9's rusty_naner facet, `platform::term::
// ConsoleAcquisition`): `AllocConsole`/`FreeConsole`/`AttachConsole` are
// the GUI-subsystem attach-vs-alloc personality itself;
// `ATTACH_PARENT_PROCESS` is `AttachConsole`'s documented sentinel for
// "my own parent's console"; `SetStdHandle` is the fixup step that
// repoints this process's own std slots at the freshly (re)opened
// console handles `reopen_std_handles` obtains via `CreateFileW`
// (already admitted above) against the `CONIN$`/`CONOUT$` well-known
// device names (Windows has no third "CONERR$" name — stdout and stderr
// are two independent handles onto the same `CONOUT$` screen buffer,
// opened with two separate calls), matching the standard Win32 pattern
// (and `rusty_win32`'s own test-only `open_console` helper, promoted
// here to production code); `GENERIC_READ`/`GENERIC_WRITE` are that
// `CreateFileW` call's access-rights argument. No `GetConsoleWindow`:
// it was the initial-state probe's first version and windows-latest CI
// caught why that was wrong — see `sys::console::has_console`'s own
// doc comment for the real ConPTY-hosted-console false negative it
// produced.
pub use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
pub use windows_sys::Win32::System::Console::{
    AllocConsole, AttachConsole, FreeConsole, SetStdHandle, ATTACH_PARENT_PROCESS,
};
pub use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
pub use windows_sys::Win32::System::Pipes::CreatePipe;
pub use windows_sys::Win32::System::Threading::{
    CreateProcessW, GetCurrentProcess, GetExitCodeProcess, ResumeThread, TerminateProcess,
    WaitForMultipleObjects, WaitForSingleObject, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    INFINITE, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOW,
};
// `Spawner::adopt` (rustils#47): opening a process this backend did not
// itself spawn, by pid — `OpenProcess` is the one Win32 call that does
// that at all (`CreateProcessW` always creates a fresh process; nothing
// else in this file's existing admissions can turn a bare `u32` into a
// HANDLE). `PROCESS_SET_QUOTA | PROCESS_TERMINATE` is exactly what
// `AssignProcessToJobObject` (already admitted below) and
// `TerminateProcess` (already admitted above) each require — no broader
// access than the two calls this handle is actually used for.
// `condrv::ClientProcess` (#1259) opens a console client by the pid its
// CONNECT carries with `PROCESS_TERMINATE | SYNCHRONIZE` (both admitted),
// to end it after the close grace with `TerminateProcess` unless a
// zero-timeout `WaitForSingleObject` shows it has already exited.
pub use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
};
// `condrv::ClientProcess::open` (#1259, D2 admission proposed on #1286):
// ConDrv doesn't keep a client waiting on its CONNECT referenced, so its
// pid can be freed and reused before the host opens it. The CONNECT also
// carries the client's thread id; `OpenThread` with
// `THREAD_QUERY_LIMITED_INFORMATION`, the least access
// `GetProcessIdOfThread` needs, pins that thread and its process while
// the host checks the thread belongs to the pid, then opens the pid.
pub use windows_sys::Win32::System::Threading::{
    GetProcessIdOfThread, OpenThread, THREAD_QUERY_LIMITED_INFORMATION,
};
// `Spawner::is_alive` (`docs/decision-request-detach-liveness.md`):
// `PROCESS_QUERY_LIMITED_INFORMATION` is the least-privileged access
// mask that still lets `GetExitCodeProcess` (already admitted above)
// answer "is this pid still running" for an arbitrary pid, including
// ones opened at a lower privilege than this process's own; narrower
// than `adopt`'s `PROCESS_SET_QUOTA | PROCESS_TERMINATE` above, which
// this query has no need of.
pub use windows_sys::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION;
// `Command::detach` (`docs/decision-request-detach-liveness.md`):
// `CREATE_NEW_PROCESS_GROUP` takes the child out of this process's
// Ctrl-C group (so a console close/Ctrl-C here doesn't propagate);
// `DETACHED_PROCESS` gives it no console at all. Both are plain
// `CreateProcessW` `dwCreationFlags` bits, freely composable with
// `CREATE_SUSPENDED`/`CREATE_UNICODE_ENVIRONMENT` (already admitted
// above) — no new Win32 subsystem, no interaction with Job Objects.
pub use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS};
pub use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
// Oracle for the winargv tests only: parse a command line the way MSVCRT
// argv splitting does, to round-trip what `winargv` builds. Not used by
// backend code.
pub use windows_sys::Win32::Foundation::LocalFree;
pub use windows_sys::Win32::UI::Shell::CommandLineToArgvW;

// Net surface, TCP slice (RFC v2 R5+, D16). Winsock is a distinct
// subsystem from every other admission above: it needs its own
// process-lifetime init/teardown (`WSAStartup`/`WSACleanup`, called
// once and refcounted — `sys::net`'s doc comment has the lifecycle
// story) and its own error-code space (`WSAGetLastError`, not
// `GetLastError`).
pub use windows_sys::Win32::Networking::WinSock::{
    accept, bind, closesocket, connect, getpeername, getsockname, listen, recv, send, setsockopt,
    socket, WSACleanup, WSAGetLastError, WSAStartup, AF_INET, AF_INET6, INVALID_SOCKET, IN_ADDR,
    IN_ADDR_0, IPPROTO_TCP, SOCKADDR, SOCKADDR_IN, SOCKADDR_IN6, SOCKADDR_IN6_0, SOCKET,
    SOCK_STREAM, SOL_SOCKET, SOMAXCONN, SO_REUSEADDR, TCP_NODELAY, WSADATA, WSAEACCES,
    WSAEADDRINUSE, WSAEADDRNOTAVAIL, WSAECONNABORTED, WSAECONNREFUSED, WSAECONNRESET, WSAEINTR,
    WSAEINVAL, WSAENOBUFS, WSAENOTCONN, WSAETIMEDOUT, WSAEWOULDBLOCK,
};
pub use windows_sys::Win32::Networking::WinSock::{IN6_ADDR, IN6_ADDR_0};
// Net surface, Unix domain socket slice (RFC v2 R5+, D16 follow-on).
// `afunix.h`'s `AF_UNIX` has ridden along in Winsock since Windows 10
// 1803 — same `socket`/`bind`/`connect`/`listen`/`accept` calls above,
// just a different address family and a `SOCKADDR_UN` in place of
// `SOCKADDR_IN`/`SOCKADDR_IN6`. No new Winsock feature needed; already
// covered by `Win32_Networking_WinSock`.
pub use windows_sys::Win32::Networking::WinSock::{AF_UNIX, SOCKADDR_UN};
// Stale-cleanup bind's unlink step: an ambient-path delete, the same
// carve-out `sys::net`'s `unix_listen` already makes for `bind`/`connect`
// taking a raw `&Path` rather than a `Dir`-capability-relative name —
// `AF_UNIX` addressing is inherently ambient, unlike the Fs backend's
// single capability-rooted entry point (`open_ambient_dir`).
pub use windows_sys::Win32::Storage::FileSystem::DeleteFileW;
// Net surface, UDP datagram slice (RFC v2 R5+, D16, final slice) —
// rusty_tail's magicsock. `recvfrom`/`sendto` are UDP's connectionless
// counterpart to `recv`/`send`: the peer address travels with every
// call instead of being fixed once at `connect`/`accept` time.
pub use windows_sys::Win32::Networking::WinSock::{recvfrom, sendto, SOCK_DGRAM};
// TcpStream::set_read_timeout (rusty_rdp convergence forcing consumer —
// see platform/src/net.rs's doc comment). Winsock's `SO_RCVTIMEO` takes
// a plain millisecond `DWORD`, unlike Linux's `struct timeval` — a wire
// representation difference, not a behavior one, so not a registered
// divergence.
pub use windows_sys::Win32::Networking::WinSock::SO_RCVTIMEO;
// Raw-socket-handle + non-blocking escape hatch (rustils#59, mirroring
// the rustils#41/#42 Linux precedent): `ioctlsocket(FIONBIO, ...)` is
// Winsock's equivalent of `fcntl(F_SETFL, O_NONBLOCK)` — forced by
// rusty_tail's `rusty_tokio` scoping a Windows/IOCP reactor backend
// (`rusty_tokio#6`), the same consumer #41/#48 already served on
// Linux/macOS.
pub use windows_sys::Win32::Networking::WinSock::{ioctlsocket, FIONBIO};
// `WindowsUnixStream::{try_clone, shutdown}` (rusty_mux's attach client,
// which reads and writes one connection from two threads and ends the
// reading one on detach): std's own `Socket::duplicate` recipe
// (`WSADuplicateSocketW` for this process, then `WSASocketW` from the
// protocol info) and `shutdown`.
pub use windows_sys::Win32::Networking::WinSock::{
    shutdown, WSADuplicateSocketW, WSASocketW, FROM_PROTOCOL_INFO, SD_BOTH, SD_RECEIVE, SD_SEND,
    WSAPROTOCOL_INFOW, WSA_FLAG_NO_HANDLE_INHERIT, WSA_FLAG_OVERLAPPED,
};
pub use windows_sys::Win32::System::Threading::GetCurrentProcessId;
// `console::poll_readable` drops queued input records that carry no
// character (key-ups, bare modifier presses, focus and size events), so
// "readable" means a read will return a byte; `VkKeyScanW(0)` names the
// key that types NUL, as conhost decides it. `console::read_chunk` reads
// a console with `ReadConsoleW` and encodes UTF-8 itself, so the input
// code page never applies. `WriteConsoleInputW` and the code-page pair
// are how its tests queue records under a non-UTF-8 code page.
pub use windows_sys::Win32::System::Console::{
    GetConsoleCP, PeekConsoleInputW, ReadConsoleInputW, ReadConsoleW, SetConsoleCP,
    WriteConsoleInputW, INPUT_RECORD, INPUT_RECORD_0, KEY_EVENT, KEY_EVENT_RECORD,
    KEY_EVENT_RECORD_0, LEFT_ALT_PRESSED, LEFT_CTRL_PRESSED, RIGHT_ALT_PRESSED, RIGHT_CTRL_PRESSED,
    SHIFT_PRESSED,
};
pub use windows_sys::Win32::UI::Input::KeyboardAndMouse::VkKeyScanW;
// `condrv::{is_valid_code_page, decode, encode}`: our console host's
// code pages (#1222 PR 9). conhost converts the `A` APIs' text with the
// same three calls, without `MB_ERR_INVALID_CHARS`.
pub use windows_sys::Win32::Globalization::{
    IsValidCodePage, MultiByteToWideChar, WideCharToMultiByte,
};

// Security surface, CSPRNG slice (RFC v2 R5+, D15, first slice) —
// rusty_rdp's five hand-rolled `/dev/urandom` reads. `BCryptGenRandom`
// with the system preferred RNG (`BCRYPT_USE_SYSTEM_PREFERRED_RNG`,
// which requires a null algorithm handle) is the modern replacement for
// the deprecated `CryptGenRandom` — Windows' equivalent CSPRNG entry
// point, mirroring Linux's `getrandom(2)`.
pub use windows_sys::Win32::Security::Cryptography::{
    BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
};

// Security surface, TrustAnchors slice (rustils#88): enumerate the
// system ROOT certificate store. `CertOpenSystemStoreW(NULL, "ROOT")`
// opens it, `CertEnumCertificatesInStore` walks it (returning a borrowed
// `CERT_CONTEXT` per call, NULL to end), `CertCloseStore` releases it.
// `CERT_CONTEXT` is the struct whose `pbCertEncoded`/`cbCertEncoded`
// carry the DER this slice hands back — read only, never interpreted
// here (`platform::security::TrustAnchors`).
pub use windows_sys::Win32::Security::Cryptography::{
    CertCloseStore, CertEnumCertificatesInStore, CertOpenSystemStoreW, CERT_CONTEXT,
};

// Security surface, CredentialStore slice (RFC v2 R5+, D15, Phase 6 item
// 2 — rustils#76): Windows' native secret store, Credential Manager.
// `CredWriteW`/`CredReadW`/`CredFree` are the write/read/free triad;
// `CRED_TYPE_GENERIC` is the generic (non-domain-password) credential
// type every non-Windows-logon consumer uses; `CRED_PERSIST_LOCAL_MACHINE`
// persists beyond the current logon session, the durability a "store
// this secret" caller expects (matching how browsers/git-credential-
// manager persist saved credentials). No `CredDeleteW` admitted — this
// slice has no `delete` operation (rustils#76's own scope note), and
// `CredWriteW` already replaces an existing credential of the same
// `TargetName`/`Type` in place, which is all `CredentialStore::set`'s
// "replace" contract needs.
pub use windows_sys::Win32::Security::Credentials::{
    CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
};
// `ERROR_NOT_FOUND`: `CredReadW`'s "no credential under this TargetName"
// code — distinct from `ERROR_FILE_NOT_FOUND` (already admitted above
// for the Fs surface), Credential Manager's own not-found signal.
pub use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;

// PTY surface, Windows ConPTY backend (RFC v2 R5+, D13, convergence
// roadmap Phase 7, rustils#83 — part 2/2, following #82's Linux
// backend). `CreatePseudoConsole`/`ClosePseudoConsole`/
// `ResizePseudoConsole`/`HPCON`/`COORD` are the pseudo-console API
// itself. `InitializeProcThreadAttributeList`/`UpdateProcThreadAttribute`/
// `DeleteProcThreadAttributeList`/`LPPROC_THREAD_ATTRIBUTE_LIST`/
// `STARTUPINFOEXW`/`EXTENDED_STARTUPINFO_PRESENT`/
// `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` are what wires the pseudo console
// to a child at `CreateProcessW` time — the only way to attach one on
// Windows (`docs/design-discussion-pty.md`'s "shape question" section:
// there is no Win32 call to attach a pseudo console after the fact, the
// reason `platform::pty::Pty::spawn` is one atomic operation rather than
// separable open/attach steps). `PeekNamedPipe` lets the teardown
// sequence drain the master's output pipe without blocking before
// calling `ClosePseudoConsole` — `ClosePseudoConsole` itself blocks
// until conhost's internal writer has finished, which can deadlock
// against an un-drained pipe (`docs/design-discussion-pty.md`'s
// EOF-vs-exit teardown lesson). `ReadFile`/`WriteFile` for the master's
// own I/O are already admitted above (Fs surface) — a ConPTY pipe handle
// takes them identically to any other pipe handle, `ERROR_BROKEN_PIPE`
// included (`sys::fileio::read`'s existing EOF translation applies
// unchanged).
pub use windows_sys::Win32::System::Console::{
    ClosePseudoConsole, CreatePseudoConsole, ResizePseudoConsole, COORD, HPCON,
};
pub use windows_sys::Win32::System::Pipes::PeekNamedPipe;
pub use windows_sys::Win32::System::Threading::{
    DeleteProcThreadAttributeList, InitializeProcThreadAttributeList, UpdateProcThreadAttribute,
    EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, STARTUPINFOEXW,
};
// `ERROR_INSUFFICIENT_BUFFER`: `InitializeProcThreadAttributeList`'s
// documented "this was only a size query" failure on its first,
// size-discovering call — expected, not a real error (checked
// specifically so a genuinely different failure there isn't silently
// papered over).
pub use windows_sys::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;

// Peer identity and owner-only socket files (rusty_mux on Windows):
// `sys::security::UserSid`, `sys::security::restrict_to_owner` and
// `sys::net::unix_peer_pid`, plus the DACL read-back the tests use.
pub use windows_sys::Win32::Networking::WinSock::{WSAIoctl, SIO_AF_UNIX_GETPEERPID};
pub use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT,
};
pub use windows_sys::Win32::Security::{
    AclSizeInformation, EqualSid, GetAce, GetAclInformation, GetSecurityDescriptorControl,
    GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, ACCESS_ALLOWED_ACE, ACL,
    ACL_SIZE_INFORMATION, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER,
};
pub use windows_sys::Win32::Storage::FileSystem::{
    FILE_ALL_ACCESS, FILE_FLAG_OPEN_REPARSE_POINT, READ_CONTROL, WRITE_DAC,
};
pub use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;
pub use windows_sys::Win32::System::Threading::OpenProcessToken;

// Console host surface (`sys::condrv`, #1222 PR 1; design
// `docs/projects/term/conhost/design.md` §11.1). Our own host serves the
// console driver (ConDrv) directly instead of going through ConPTY:
// - `LoadLibraryExW` with `LOAD_LIBRARY_SEARCH_SYSTEM32` and
//   `GetProcAddress` resolve the undocumented `user32!ConsoleControl` at
//   runtime, from System32 only, so a build without it degrades to
//   `ErrorKind::Unsupported` instead of failing to load. It is never
//   linked statically.
// - The server handle is asynchronous: every IOCTL on it is overlapped
//   (`OVERLAPPED`, `GetOverlappedResult`), each with its own event
//   (`CreateEventW`, `ResetEvent`), and `CancelIoEx` requests
//   cancellation of the one in-flight READ_IO. `SetEvent` drives the
//   waker and the input-available event clients wait on.
// - `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` limits what the attached child
//   inherits to its three std handles.
// - The `ERROR_*` and `WAIT_FAILED` constants are those calls' documented
//   results: a pending overlapped call, a cancelled one, and a driver
//   that has dropped the server.
pub use windows_sys::Win32::Foundation::{
    ERROR_IO_PENDING, ERROR_OPERATION_ABORTED, ERROR_PIPE_NOT_CONNECTED, FARPROC, WAIT_FAILED,
};
pub use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
pub use windows_sys::Win32::System::Threading::{
    CreateEventW, ResetEvent, SetEvent, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
};
pub use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

// Console client calls for the console host's parity probes
// (`rusty_conhost`'s `conhost_probe` example; design
// `docs/projects/term/conhost/design.md` §8.1, Phase B in §9.2). A probe
// is a real console client: it calls each API through kernelbase, as
// the programs a console serves do, so the same call can be compared
// under ConPTY and under our host. No backend module calls these; the
// feature (`Win32_System_Console`) is already enabled.
pub use windows_sys::Win32::System::Console::{
    FillConsoleOutputAttribute, FillConsoleOutputCharacterW, FlushConsoleInputBuffer,
    GenerateConsoleCtrlEvent, GetConsoleCursorInfo, GetConsoleOutputCP, GetConsoleProcessList,
    GetConsoleScreenBufferInfoEx, GetConsoleTitleW, GetConsoleWindow, GetCurrentConsoleFont,
    GetLargestConsoleWindowSize, GetNumberOfConsoleInputEvents, ReadConsoleOutputAttribute,
    ReadConsoleOutputCharacterW, ReadConsoleOutputW, ScrollConsoleScreenBufferW,
    SetConsoleCursorInfo, SetConsoleCursorPosition, SetConsoleOutputCP, SetConsoleScreenBufferSize,
    SetConsoleTextAttribute, SetConsoleTitleW, SetConsoleWindowInfo, WriteConsoleA,
    WriteConsoleOutputAttribute, WriteConsoleOutputCharacterW, WriteConsoleOutputW, WriteConsoleW,
    CHAR_INFO, CHAR_INFO_0, CONSOLE_CURSOR_INFO, CONSOLE_FONT_INFO, CONSOLE_SCREEN_BUFFER_INFOEX,
    SMALL_RECT,
};

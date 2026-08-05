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
// "my own parent's console"; `GetConsoleWindow` backs the initial-state
// probe (a non-null `HWND` means a console is already attached, the
// `ConsoleState::Inherited` case) since there is no direct "do I have a
// console" query otherwise; `SetStdHandle` is the fixup step that
// repoints this process's own std slots at the freshly (re)opened
// console handles `reopen_std_handles` obtains via `CreateFileW`
// (already admitted above) against the `CONIN$`/`CONOUT$` well-known
// device names (Windows has no third "CONERR$" name — stdout and stderr
// are two independent handles onto the same `CONOUT$` screen buffer,
// opened with two separate calls), matching the standard Win32 pattern
// (and `rusty_win32`'s own test-only `open_console` helper, promoted
// here to production code); `GENERIC_READ`/`GENERIC_WRITE` are that
// `CreateFileW` call's access-rights argument.
pub use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
pub use windows_sys::Win32::System::Console::{
    AllocConsole, AttachConsole, FreeConsole, GetConsoleWindow, SetStdHandle, ATTACH_PARENT_PROCESS,
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
pub use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
};
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
    WSAEINVAL, WSAENOTCONN, WSAETIMEDOUT, WSAEWOULDBLOCK,
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

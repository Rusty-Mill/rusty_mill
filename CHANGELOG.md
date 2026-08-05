# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
- `conpty::create_with_flags` plus `conpty::PSEUDOCONSOLE_INHERIT_CURSOR` —
  closes issue #263, a parity-loop gap: `conpty::create` hardcoded
  `CreatePseudoConsole`'s `dwFlags` to `0`, so the only flag Windows
  documents for it (inherit the parent terminal's current cursor position
  rather than starting at `(0,0)`) could never be requested. Added
  alongside `create`, which now forwards to it with `flags = 0` — not a
  signature change.

### Changed
- `README.md`'s "Status" section rewritten to match the crate's actual
  current module set (it had drifted as far back as "Phase 5" while the
  crate grew round-2 subsystems, a parity-loop sweep, `conpty`, and `pe`).
  `gap-analysis.md` and `docs/CAPABILITY_ASSESSMENT.md` — both fully
  closed out, with every item shipped or consciously deferred — moved to
  `docs/archive/` with an archival banner, so neither reads as a live
  backlog; all in-repo references to their old paths updated to match.

### Added
- `pe` module (new subsystem): `pe::PeFile` — a zero-copy,
  allocation-free parser for the on-disk layout of a Portable Executable
  image, the read half of a PE loader. Exposes the target `Machine`,
  `Subsystem` (console vs GUI), DLL-vs-executable characteristics, entry
  point, preferred image base, section table, data directories,
  `rva_to_offset` translation, and iterators over the image's `exports`
  (name + ordinal — the on-disk complement of `dynlib::get_proc_address`)
  and imported module names (`imports`). Deliberately the crate's one
  module with no Win32 export to wrap and, consequently, no `unsafe`
  anywhere in it; like `error`, it takes a `&[u8]` rather than a live
  handle and so is not `#[cfg(windows)]`-gated (a Windows binary can be
  inspected from any host). Stops short of the *other* half of loading
  (mapping/relocating/running an image in-process), which stays the OS
  loader's job via `process`/`dynlib`.
- `process::spawn_suspended_with_pseudoconsole` plus `StartupInfoExW`/
  `EXTENDED_STARTUPINFO_PRESENT` — a wholly new function alongside
  `process::spawn_suspended` (non-breaking) that starts a command
  suspended, hosted by an already-created pseudoconsole, for a fully-
  interactive child rather than one with redirected stdio pipes. This
  closes out the `conpty` module's round-2 scope, and with it this
  entire round-2 parity-loop batch.
- `conpty::AttributeList` (`InitializeProcThreadAttributeList`/
  `UpdateProcThreadAttribute`/`DeleteProcThreadAttributeList`) — the
  generic extended-process-attribute mechanism `CreateProcessW` reads to
  receive an `Hpcon`. A query-then-allocate opaque-byte-buffer pattern,
  new territory for this crate, distinct from its existing "retry a
  UTF-16 buffer at the reported size" idiom.
- `conpty` module (new subsystem): `conpty::create`/`conpty::resize`/
  `conpty::close` (`CreatePseudoConsole`/`ResizePseudoConsole`/
  `ClosePseudoConsole`) plus `Hpcon` and
  `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` — the pseudoconsole lifecycle,
  the first piece of ConPTY support. `Hpcon`/`PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`
  were originally a separate issue, folded in here since `create`'s own
  signature already needs the `Hpcon` type.
- `net::htons`/`net::htonl`/`net::ntohs`/`net::ntohl` — thin wrappers
  over the Winsock functions of the same name, for callers building or
  parsing their own raw wire fields rather than going through
  `SocketAddr`. This rounds out the `net` module's round-2 scope.
- `net::resolve` (`getaddrinfo`/`freeaddrinfo`) plus `AddrInfoHints`/
  `ResolvedAddr` — hostname/service-name to address resolution, walking
  Windows' own `addrinfo` linked list and copying every entry out before
  freeing it. Accepts numeric or real host/service names; entries with
  an address family/socket type/protocol this module doesn't support are
  silently skipped rather than erroring the whole call.
- `net::local_addr`/`net::peer_addr` (`getsockname`/`getpeername`) —
  read back a socket's own bound address (e.g. after binding an
  ephemeral port `0`) and a connected socket's peer address, reusing the
  existing `from_sockaddr` decoding.
- `net::set_sockopt`/`net::get_sockopt` (`setsockopt`/`getsockopt`) plus
  `SockOpt`/`SockOptKind`/`SockOptValue` — `SO_REUSEADDR`, `SO_RCVTIMEO`/
  `SO_SNDTIMEO` (a plain millisecond `DWORD` on Windows, unlike POSIX's
  `timeval`), `TCP_NODELAY`, and `SO_ERROR`.
- `net::shutdown` (`shutdown`) plus `ShutdownHow` (`Receive`/`Send`/`Both`)
  — half-close a connected socket's send and/or receive direction without
  invalidating the handle itself, unlike `close_socket`. Lets a TCP client
  signal end-of-stream to its peer while still able to `recv` any
  in-flight reply.
- `net::sendto`/`net::recvfrom` (`sendto`/`recvfrom`) — connectionless
  datagram I/O, the bare UDP round trip, marshaling a
  `sockaddr_in`/`sockaddr_in6` address (via the existing
  `to_sockaddr`/`from_sockaddr` plumbing) on every call rather than
  relying on a fixed peer.
- `net::send`/`net::recv` (`send`/`recv`) — byte-slice I/O on a
  connected socket, thin wrappers matching `console::read`/
  `console::write`'s shape. `recv` reports `Ok(0)` for the peer's
  orderly shutdown, not an error (the TCP analog of `ReadFile`
  reporting end-of-file).
- `net::connect` (`connect`) — TCP client connect, or fix a UDP socket's
  default peer for future `send`/`recv` calls.
- `net::accept` (`accept`) — accept one incoming TCP connection,
  returning a new connected socket plus the peer's address. The first
  real (non-test) caller of `from_sockaddr`.
- `net::listen` (`listen`) — mark a bound TCP socket passive/listening,
  needed before it can accept incoming connections.
- `SocketAddr` (`V4`/`V6`) plus `to_sockaddr`/`from_sockaddr` conversions
  and the verified `sockaddr_in`/`sockaddr_in6` wire-format layouts (16
  and 28 bytes) — the shared `{ip, port}` address plumbing every
  address-taking `net` function needs. Implemented ahead of its own
  filed order since `net::bind` (below) already needed it.
- `net::bind` (`bind`) — attach a local address/port to a socket, needed
  before a socket can accept connections/datagrams on a specific address
  or send from a fixed source port.
- `net::socket`/`net::close_socket` (`socket`/`closesocket`) plus
  `RawSocket`, `AddressFamily` (`Inet`/`Inet6`), `SocketKind`
  (`Stream`/`Dgram`), and `Protocol` (`Tcp`/`Udp`) — socket lifecycle
  create/destroy. `RawSocket = usize` is a distinct handle namespace
  from `handle::RawHandle`: a `SOCKET` is closed via `close_socket`,
  never `CloseHandle`.
- `net` module (new subsystem): `net::startup`/`net::cleanup`
  (`WSAStartup`/`WSACleanup`) — Winsock's own load/unload lifecycle, the
  one primitive with no POSIX/`rusty_libc` analog (every other Winsock
  call is documented undefined behavior before a matching `WSAStartup`
  or after `WSACleanup`). Windows reference-counts nested calls
  internally, so no shared guard/RAII type is needed. Previously
  excluded by this crate's own non-goals, now in scope per explicit
  round-2 direction — first piece of basic TCP/UDP client+server socket
  programming, the same core subset `rusty_libc` wraps for POSIX
  sockets.
- `service::dependent_services` (`EnumDependentServicesW`) plus
  `DependentService`/`ServiceState`/`SERVICE_ENUMERATE_DEPENDENTS` —
  list every service depending on a given one, the "will stopping this
  break something else" check before calling `control` with
  `ServiceControl::Stop`. This completes the `service` module's round-2
  batch (issues #165-#172).
- `service::display_name`/`service::key_name`
  (`GetServiceDisplayNameW`/`GetServiceKeyNameW`) — translate between a
  service's short key name (`"eventlog"`) and human-readable display name
  (`"Windows Event Log"`), using the query-size-then-allocate idiom this
  crate already uses elsewhere.
- `service::config` (`QueryServiceConfigW`) plus `ServiceConfig` — one
  service's static configuration (start type, binary path, display name,
  dependencies), for a `systemctl show`-style detail view. Grows the
  buffer on `ERROR_INSUFFICIENT_BUFFER`, needing only one retry since
  `QueryServiceConfigW` reports the exact size up front. `dependencies`
  decodes `lpDependencies`'s `REG_MULTI_SZ`-shaped list into an owned
  `Vec<String>`.
- `service::control` (`ControlService`) plus `ServiceControl`
  (`Stop`/`Pause`/`Continue`/`Interrogate`) and `SERVICE_INTERROGATE` —
  send a stop/pause/continue/interrogate control to a running service.
  Discards `ControlService`'s own immediate status out-parameter (it
  only reports the state at the instant of the call, often not yet
  settled) — a caller polls `status` afterward instead, the same
  poll-don't-block shape `job::process_ids` already uses.
- `service::start` (`StartServiceW`) plus `SERVICE_START` — start an
  already-installed service, the zero-argument case only
  (`lpServiceArgVectors` only matters for driver-style services, out of
  scope). Also adds `Win32Error::ERROR_SERVICE_ALREADY_RUNNING`, needed
  for `start`'s already-running error path.
- `service::status` (`QueryServiceStatusEx`, `SC_STATUS_PROCESS_INFO`)
  plus `ServiceStatus` and the seven `SERVICE_STOPPED`/
  `SERVICE_START_PENDING`/`SERVICE_STOP_PENDING`/`SERVICE_RUNNING`/
  `SERVICE_CONTINUE_PENDING`/`SERVICE_PAUSE_PENDING`/`SERVICE_PAUSED`
  state constants — one named service's live status including its
  backing process id, superseding the older, pid-less
  `QueryServiceStatus`.
- `service::enum_services` (`EnumServicesStatusExW`) plus
  `ServiceStatusEntry` and `SC_MANAGER_ENUMERATE_SERVICE`/`SERVICE_WIN32`/
  `SERVICE_ACTIVE`/`SERVICE_INACTIVE`/`SERVICE_STATE_ALL` — list every
  service known to the SCM with its current status, the core of a
  `systemctl list-units`-equivalent. Pages internally via
  `EnumServicesStatusExW`'s own resume-handle protocol, growing the
  buffer only when a page can't fit even one entry.
- `service` module (new subsystem): `service::open_manager`/
  `service::open_service`/`service::close` (`OpenSCManagerW`/
  `OpenServiceW`/`CloseServiceHandle`) plus `SC_MANAGER_CONNECT`/
  `SERVICE_QUERY_CONFIG`/`SERVICE_QUERY_STATUS` — the SCM/service handle
  lifecycle, first piece of a `systemctl`-equivalent, previously excluded
  by this crate's own non-goals, now in scope per explicit round-2
  direction. Reuses `handle::RawHandle` for `SC_HANDLE` rather than a
  distinct type (ABI-compatible, `DECLARE_HANDLE`-based like `HANDLE`
  itself), closed via `service::close`/`CloseServiceHandle`, never
  `handle`'s own close functions.
- `security::sd_to_string`/`security::string_to_sd`
  (`ConvertSecurityDescriptorToStringSecurityDescriptorW`/
  `ConvertStringSecurityDescriptorToSecurityDescriptorW`) plus
  `ConvertedSecurityDescriptor` (freeing via `LocalFree` on `Drop`) and a
  new `PathSecurityInfo::raw_security_descriptor` accessor — a
  debug/snapshot (`icacls /save`-style) SDDL string representation of a
  security descriptor's full permission state. This completes the
  `security` module's first round-2 batch (issues #154-#164).
- `security::well_known_sid` (`CreateWellKnownSid`) plus `WellKnownSidType`
  (`Everyone`/`LocalSystem`/`BuiltinAdministrators`) — construct a
  well-known SID without a name-lookup round trip.
- `security::sid_equal` (`EqualSid`) — byte-correct SID comparison; a
  naive memory comparison isn't safe here since a `PSID`'s trailing
  sub-authority count varies its total size.
- `security::sid_length`/`security::is_valid_sid`/`security::copy_sid`
  (`GetLengthSid`/`IsValidSid`/`CopySid`) — sizing, validity-checking, and
  owned-copying of an opaque `PSID`, needed anywhere a SID must outlive
  the short-lived buffer it was originally borrowed from (e.g. an
  owner/ACE SID from `path_security_info`/`acl_entries`). `copy_sid`
  reuses the existing `SidBuf` type rather than introducing a new one.
- `security::initialize_acl`/`security::add_access_allowed_ace`/
  `security::add_access_denied_ace` (`InitializeAcl`/
  `AddAccessAllowedAce`/`AddAccessDeniedAce`) — the lower-level,
  per-ACE alternative to `build_acl`'s all-at-once `SetEntriesInAclW`,
  useful for building a brand-new object's initial ACL from scratch.
- `security::sid_to_string`/`security::string_to_sid`
  (`ConvertSidToStringSidW`/`ConvertStringSidToSidW`) plus `ConvertedSid`
  (freeing via `LocalFree` on `Drop`) — the `S-1-5-...` string form of a
  SID, the fallback `icacls` itself uses when a SID can't be resolved to
  a name (orphaned/foreign/deleted account).
- `security::lookup_account_sid`/`security::lookup_account_name`
  (`LookupAccountSidW`/`LookupAccountNameW`) plus `AccountName`/
  `SidNameUse`/`SidBuf` — SID↔name resolution, turning an owner/ACE SID
  into a `"DOMAIN\name"` display string and a name into the SID
  `build_trustee_with_sid`/`build_acl` need.
- `security::build_trustee_with_sid`/`security::build_trustee_with_name`
  (`BuildTrusteeWithSidW`/`BuildTrusteeWithNameW`) — wrap a `PSID` or a
  wide-string name into the `Trustee` shape `build_acl`'s entries need.
  Supersedes the crate-internal `Trustee::from_sid` helper added
  alongside `build_acl` (removed now that the real Win32 primitive
  covers the same ground).
- `security::build_acl` (`SetEntriesInAclW`) plus `ExplicitAccess`/
  `Trustee`/`AccessMode`/`TrusteeForm`/`TrusteeType` (real, fully-fielded
  FFI mirrors — unlike `Acl`/`PSID`, these are genuinely fixed-size) and
  `BuiltAcl` (freeing via `LocalFree` on `Drop`) — build a new ACL from
  an existing one plus add/replace/remove entries, the primitive behind
  `icacls /grant`/`/deny`.
- `security::acl_entries` (`GetAclInformation` + `GetAce`) plus
  `AclEntry`/`AceKind` and a fixed-header-only `Acl` mirror — enumerate a
  DACL/SACL's ACEs one at a time, turning an opaque ACL into the
  human-readable permission list `icacls`/`ls -l` displays.
- `security` module (new subsystem): `security::path_security_info`/
  `security::set_path_security_info` (`GetNamedSecurityInfoW`/
  `SetNamedSecurityInfoW`, freeing via `LocalFree` on `Drop`) plus the
  `PathSecurityInfo` type and `SecurityInfoFlags` (`OWNER_*`/`GROUP_*`/
  `DACL_SECURITY_INFORMATION`) — the core path → owner `PSID`/DACL `PACL`
  round trip, first piece of file/directory security inspection and
  modification, previously excluded by this crate's own non-goals, now
  in scope per explicit round-2 direction.
- `registry::delete_tree` (`RegDeleteTreeW`) — recursively delete a
  subkey and everything beneath it in one call, without
  `delete_key`'s leaf-only restriction forcing a hand-rolled
  enumerate-and-recurse loop. This completes the `registry` module's
  round-2 item list (issues #142-#153).
- `registry::flush_key` (`RegFlushKey`) — force a key's changes to disk
  immediately instead of Windows' lazy flush, a real durability gap for
  settings writes right before a risky operation.
- `registry::key_info` (`RegQueryInfoKeyW`) plus the `KeyInfo` struct —
  subkey/value counts and max name/data lengths in one call, the same
  query `enum_values`/`enum_keys` already use internally to pre-size
  their own buffers, now exposed directly.
- `registry::enum_keys` (`RegEnumKeyExW`, sized via `RegQueryInfoKeyW`)
  plus the `RegKeyIter` iterator type — enumerate a key's immediate
  subkey names as `(String, Timespec)` pairs, the last-write time decoded
  the same "raw `FILETIME` mirror stays private" way
  `process::times`/`fs::stat` etc. already do.
- `registry::enum_values` (`RegEnumValueW`, sized via `RegQueryInfoKeyW`)
  plus the `RegValueIter` iterator type and `Win32Error::ERROR_NO_MORE_ITEMS`
  — enumerate every value under a key as `(String, RegistryValue)` pairs.
- `registry::delete_key` (`RegDeleteKeyExW`) plus `KEY_WOW64_64KEY`/
  `KEY_WOW64_32KEY` — remove a leaf subkey (must have no subkeys of its
  own). Earlier registry tests now clean up the keys they create, now
  that this exists to do it.
- `registry::delete_value` (`RegDeleteValueW`) — remove one named value
  under an open key, without touching the key itself or its other
  values/subkeys.
- `registry::set_value` (`RegSetValueExW`), the write-side counterpart to
  `query_value` — encodes a `RegistryValue` back into the `dwType`/
  byte-buffer shape each `REG_*` type expects.
- `registry::query_value` (`RegQueryValueExW`) plus the `RegistryValue`
  enum (`None`/`Sz`/`ExpandSz`/`Dword`/`Qword`/`Binary`/`MultiSz`) — reads
  a value's data decoded by its real `dwType`, using the
  query-size-then-allocate idiom `path::search_path`/`fs::final_path`
  already use.
- `registry::create_key` (`RegCreateKeyExW`) plus `KeyDisposition`
  (`CreatedNewKey`/`OpenedExistingKey`) — an idempotent "open or create"
  in one call, reporting via the returned disposition which one happened.
- `registry::open_key`/`registry::close_key` (`RegOpenKeyExW`/
  `RegCloseKey`) plus `KEY_READ`/`KEY_WRITE`/`KEY_ALL_ACCESS`/
  `KEY_QUERY_VALUE` REGSAM constants — opening a subkey of a predefined
  root (or another already-open key) and closing it again.
- `registry` module (new subsystem): `HKey` type plus the five predefined
  root keys (`HKEY_CLASSES_ROOT`/`HKEY_CURRENT_USER`/`HKEY_LOCAL_MACHINE`/
  `HKEY_USERS`/`HKEY_CURRENT_CONFIG`), the first piece of Windows Registry
  access — previously excluded by this crate's own non-goals, now in scope
  per explicit round-2 direction (`gap-analysis.md`).
- `console::largest_window_size`/`console::set_screen_buffer_size`/
  `console::set_window_info` (`GetLargestConsoleWindowSize`/
  `SetConsoleScreenBufferSize`/`SetConsoleWindowInfo`) plus public
  `Coord`/`SmallRect` types, the write side of console geometry
  (`window_size` was already read-only).
- `console::alloc`/`console::free`/`console::attach` (`AllocConsole`/
  `FreeConsole`/`AttachConsole`), letting a GUI-subsystem process acquire,
  release, or reattach to a console on demand (`attach(None)` maps to
  `ATTACH_PARENT_PROCESS`). This crate's own tests already used
  `AllocConsole` internally; it's now exposed as public API too.
- `process::spawn_suspended`'s `new_process_group` parameter and
  `console::generate_ctrl_event`, for interrupting one background child via
  a targeted `CTRL_BREAK_EVENT` instead of affecting the whole console.
- `process::times` (`GetProcessTimes`), CPU-time accounting for a process
  handle — creation/exit wall-clock timestamps plus kernel/user elapsed
  duration.
- `job::set_resource_limits`/`job::limits` (memory/CPU-time/active-process
  Job Object limits) and `job::accounting`
  (`JobObjectBasicAndIoAccountingInformation`), the narrow subset of `ulimit`
  a Job Object can enforce, plus lifetime CPU/IO accounting.
- `pipe` module: named pipes (`CreateNamedPipeW`/`ConnectNamedPipeW`/
  `WaitNamedPipeW`/`CreateFileW`), the primitive rush's deferred process
  substitution (`<(cmd)`) and `coproc` support need on Windows.
- `console::write_key_events` (`WriteConsoleInputW` for non-character keys —
  arrows, Home/End, function keys, …) plus the `VK_*`/`ENHANCED_KEY`
  constants it uses. First non-`kernel32` link in this crate
  (`user32.dll`'s `MapVirtualKeyW`).
- `volume` module: drive/volume enumeration (`GetLogicalDrives`/
  `GetDriveTypeW`/`GetVolumeInformationW`) — a distinctly Windows-shaped
  primitive (multi-root filesystem, no Unix analog) with no current
  consumer, added for completeness.
- `path::short_path`/`path::long_path` (`GetShortPathNameW`/
  `GetLongPathNameW`), normalizing between a legacy 8.3 short name and its
  long form.
- `watch` module: filesystem change notification (`ReadDirectoryChangesW`),
  wrapped in `OVERLAPPED` I/O with a `process::wait`-style `Option<u32>`
  timeout — this crate's first genuinely overlapped primitive, since
  `ReadDirectoryChangesW` has no other way to bound how long it blocks.
- `path::current_dir`/`path::set_current_dir` (`GetCurrentDirectoryW`/
  `SetCurrentDirectoryW`) — the actual Win32 primitives behind `cd`/`pwd`,
  found by a parity-loop pass against the real Win32 API surface
  (`gap-analysis.md`) rather than the round-2 needs-driven assessment above.
- `fs::read_dir` (`FindFirstFileW`/`FindNextFileW`/`FindClose`), a `ReadDir`
  iterator of `DirEntry` for listing a directory's contents — another
  parity-loop find.
- `process::get_env_var`/`process::set_env_var`
  (`GetEnvironmentVariableW`/`SetEnvironmentVariableW`), live single-variable
  environment access to back `export`/`unset`/single-`$VAR` reads — another
  parity-loop find.
- `handle::get_std_handle`/`handle::set_std_handle`
  (`GetStdHandle`/`SetStdHandle`) plus the `STD_*_HANDLE` slot constants —
  the primitive `process.rs`'s own `spawn_suspended` doc comment already
  described but this crate didn't yet own; another parity-loop find.
- `path::full_path` (`GetFullPathNameW`), resolving a relative path to its
  absolute form — another parity-loop find.
- `process::process_id_of` (`GetProcessId`), the reverse of `open_by_pid`'s
  pid-to-`HANDLE` mapping — another parity-loop find.
- `process::image_path` (`QueryFullProcessImageNameW`), the full executable
  path for a process handle, completing `list_processes`'s bare-filename-only
  `exe_file` — another parity-loop find.
- `job::is_in_job` (`IsProcessInJob`), checking job membership before
  `assign` to avoid a surprise failure under an ambient job (e.g. GitHub
  Actions' Windows runners) — another parity-loop find.
- `console::title`/`console::set_title`
  (`GetConsoleTitleW`/`SetConsoleTitleW`), the Windows analog of xterm's OSC
  title-setting escape sequence — another parity-loop find.
- `console::set_cursor_position` (`SetConsoleCursorPosition`), the write
  side of `window_size`'s cursor-position read — another parity-loop find.
- `console::fill_char`/`console::fill_attribute`
  (`FillConsoleOutputCharacterW`/`FillConsoleOutputAttribute`), a
  clear-to-end-of-line-style redraw primitive — another parity-loop find.
- `console::flush_input` (`FlushConsoleInputBuffer`), discarding stale
  queued keystrokes (e.g. after Ctrl-C) — another parity-loop find.
- `console::pending_input_events` (`GetNumberOfConsoleInputEvents`), a
  non-blocking queued-input-depth check — another parity-loop find.
- `pipe::disconnect_server` (`DisconnectNamedPipe`), letting a served pipe
  instance be reset and reused for a second client instead of requiring a
  fresh server — another parity-loop find.
- `handle::handle_information` (`GetHandleInformation`), the read-side
  counterpart to `set_inheritable` — another parity-loop find.
- `volume::disk_free_space` (`GetDiskFreeSpaceExW`), free/total space for a
  `df`-style builtin — another parity-loop find.
- `process::sleep_ms` (`Sleep`), the direct primitive behind a
  `sleep`/`usleep` builtin — another parity-loop find.
- `process::logical_processor_count` (`GetSystemInfo`'s
  `dwNumberOfProcessors`), the primitive behind an `nproc`-equivalent
  builtin — another parity-loop find.
- `process::computer_name` (`GetComputerNameW`), the primitive behind
  `$HOSTNAME`/a `hostname` builtin — another parity-loop find.
- `process::memory_status` (`GlobalMemoryStatusEx`), system-wide memory
  totals/load for a `free`-style builtin — another parity-loop find.
- `process::set_error_mode` (`SetErrorMode`) plus
  `SEM_FAILCRITICALERRORS`/`SEM_NOOPENFILEERRORBOX`, suppressing blocking
  GUI error dialogs that would otherwise freeze a non-interactive script
  run — another parity-loop find.
- `fs::copy_file` (`CopyFileW`), the primitive behind a `cp` builtin —
  another parity-loop find.
- `fs::move_file` (`MoveFileExW`) plus `MOVEFILE_REPLACE_EXISTING`/
  `MOVEFILE_COPY_ALLOWED`, the primitive behind an `mv` builtin (covering
  cross-volume moves, unlike `std::fs::rename` on Windows) — another
  parity-loop find.
- `fs::delete_file` (`DeleteFileW`), the primitive behind an `rm` builtin —
  another parity-loop find.
- `fs::create_directory`/`fs::remove_directory`
  (`CreateDirectoryW`/`RemoveDirectoryW`), the primitives behind
  `mkdir`/`rmdir` builtins — another parity-loop find.
- `fs::create_hard_link` (`CreateHardLinkW`), `ln` without `-s` — another
  parity-loop find.
- `path::temp_path`/`path::temp_file_name`
  (`GetTempPathW`/`GetTempFileNameW`), for heredoc scratch files or a
  `mktemp` builtin — another parity-loop find.
- `process::priority_class`/`process::set_priority_class`
  (`GetPriorityClass`/`SetPriorityClass`) plus the `*_PRIORITY_CLASS`
  constants, the Windows analog of `nice`/`renice` — another parity-loop
  find.
- `process::list_threads`/`process::open_thread`/`process::suspend_thread`
  (`Thread32First`/`Thread32Next`/`OpenThread`/`SuspendThread`) plus
  `THREAD_SUSPEND_RESUME`, the closest Windows equivalent to a
  process-wide `SIGSTOP` — another parity-loop find.
- `pipe::set_pipe_mode` (`SetNamedPipeHandleState`), non-blocking mode and
  byte/message-mode switching for named pipes — another parity-loop find.
- `handle::create_mutex`/`handle::release_mutex`
  (`CreateMutexW`/`ReleaseMutex`), the Windows analog of `flock`'s
  cross-process locking — another parity-loop find.
- `fs::lock_file`/`fs::unlock_file` (`LockFileEx`/`UnlockFileEx`), advisory
  file-level locking — the last of the 32 parity-loop finds from
  `gap-analysis.md`.
- `process::affinity`/`process::set_affinity` (`GetProcessAffinityMask`/
  `SetProcessAffinityMask`), the Windows analog of `sched_getaffinity`/
  `taskset` — first of the round-2 "weak/no clear consumer" items added per
  explicit direction (`gap-analysis.md`), not because any consumer currently
  wants it.
- `process::thread_exit_code` (`GetExitCodeThread`) plus
  `THREAD_QUERY_INFORMATION`, the thread-level counterpart to `wait`'s
  process exit code — another round-2 item.
- `process::thread_times` (`GetThreadTimes`), the thread-level counterpart
  to `process::times` — another round-2 item.
- `handle::create_semaphore`/`handle::release_semaphore`
  (`CreateSemaphoreW`/`ReleaseSemaphore`), a counting semaphore alongside
  the already-wrapped mutex — another round-2 item.
- `handle::wait_single_ex`/`handle::wait_multiple_ex`
  (`WaitForSingleObjectEx`/`WaitForMultipleObjectsEx`) plus `WaitResult`,
  alertable-wait variants of the plain waits already used throughout this
  crate — another round-2 item.
- `process::sleep_ms_ex` (`SleepEx`), the alertable-sleep variant of
  `process::sleep_ms` — another round-2 item.
- `handle::signal_and_wait` (`SignalObjectAndWait`), atomically signaling
  one synchronization object and waiting on another — another round-2
  item.
- `volume::find_volumes` (`FindFirstVolumeW`/`FindNextVolumeW`/
  `FindVolumeClose`), enumerating every volume by its stable GUID path,
  independent of drive-letter assignment — another round-2 item.
- `volume::volume_path_name` (`GetVolumePathNameW`), mapping an arbitrary
  path to the root path of the volume it's on — another round-2 item.
- `fs::compressed_file_size` (`GetCompressedFileSizeW`), the on-disk
  (NTFS-compressed) size of a file vs. `fs::stat`'s logical size —
  another round-2 item.
- `handle::same_object` (`CompareObjectHandles`), the documented-correct
  way to ask Windows whether two handle values refer to the same kernel
  object — another round-2 item. Resolved via `GetProcAddress` at call
  time rather than this crate's usual static `#[link]` import: some
  Windows SDK versions' `kernel32.lib` omits a static stub for this
  symbol even though it's a real, always-present `kernel32.dll` export,
  which fails to link (caught by CI on this crate's own `windows-latest`
  runner) rather than just failing at runtime. Also falls back to
  `KernelBase.dll` (where the function is actually implemented) if
  `GetProcAddress` against `kernel32.dll` alone doesn't resolve it — a
  second CI-caught gap on the same runner.
- `pipe::pipe_info` (`GetNamedPipeInfo`) plus a new `PipeInfo` struct, the
  read-side counterpart to `pipe::create_server`'s creation-time
  parameters — another round-2 item.
- `pipe::transact`/`pipe::call` (`TransactNamedPipe`/`CallNamedPipeW`),
  one-shot message-mode pipe transactions for a simple request-response
  protocol — another round-2 item.
- `job::open_by_name` (`OpenJobObjectW`) plus `JOB_OBJECT_ALL_ACCESS`, the
  reverse direction of `job::create`, which only ever makes anonymous
  jobs — another round-2 item.
- `process::tick_count` (`GetTickCount64`), a coarser, simpler monotonic
  counter alongside `time::now_monotonic`'s high-resolution one —
  another round-2 item.
- `path::system_directory`/`path::windows_directory`
  (`GetSystemDirectoryW`/`GetWindowsDirectoryW`), standard
  well-known-location primitives (`C:\Windows\System32`/`C:\Windows`) —
  another round-2 item.
- `process::logical_processor_information` (`GetLogicalProcessorInformation`)
  plus `LogicalProcessorInformation`/`ProcessorRelationship`, detailed CPU
  topology beyond `process::logical_processor_count`'s single number —
  another round-2 item.
- `process::add_vectored_exception_handler`/`remove_vectored_exception_handler`
  (`AddVectoredExceptionHandler`/`RemoveVectoredExceptionHandler`) and
  `process::set_unhandled_exception_filter` (`SetUnhandledExceptionFilter`),
  structured-exception-handling hooks — the closest Windows analog to
  installing a Unix `SIGSEGV`/`SIGABRT` handler — another round-2 item.
- `console::process_list` (`GetConsoleProcessList`), the pids of every
  process attached to the calling process's console — another round-2
  item.
- `console::window_handle` (`GetConsoleWindow`), the `HWND` of the console
  window attached to the calling process, if any — another round-2 item.
### Changed
- `process::spawn_suspended` takes a new `new_process_group: bool` parameter
  (breaking, pre-1.0).
- `job::process_ids` now returns `Vec<u32>` instead of `Vec<usize>`,
  matching every other pid-carrying value in this crate (breaking,
  pre-1.0).
### Fixed
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->

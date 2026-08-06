//! The exact libc items this backend is permitted to touch.
//!
//! Anything not re-exported here is out of bounds for `sys/` — widening
//! this list is a reviewed decision, which keeps the eventual Track P
//! replacement inventory honest: this file *is* the checklist.

pub use libc::{
    c_char, c_int, close, dirent, faccessat, fchmodat, fdopendir, fstatat, fsync, kill, mkdirat,
    nfds_t, openat, pid_t, pipe2, poll, pollfd, posix_spawn, posix_spawn_file_actions_addchdir_np,
    posix_spawn_file_actions_adddup2, posix_spawn_file_actions_addopen,
    posix_spawn_file_actions_destroy, posix_spawn_file_actions_init, posix_spawn_file_actions_t,
    posix_spawnattr_destroy, posix_spawnattr_init, posix_spawnattr_setflags,
    posix_spawnattr_setpgroup, posix_spawnattr_t, read, readdir, readlinkat, sighandler_t, signal,
    stat, symlinkat, syscall, tcgetattr, tcsetattr, termios, unlinkat, waitpid, winsize, write,
    SYS_pidfd_open, SYS_renameat2, AT_FDCWD, AT_REMOVEDIR, AT_SYMLINK_NOFOLLOW, DIR, DT_DIR,
    DT_LNK, DT_REG, O_APPEND, O_CLOEXEC, O_CREAT, O_DIRECTORY, O_EXCL, O_RDONLY, O_RDWR, O_TRUNC,
    O_WRONLY, POLLIN, POSIX_SPAWN_SETPGROUP, RENAME_NOREPLACE, R_OK, SIGHUP, SIGINT, SIGKILL,
    SIGTERM, SIG_ERR, STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO, S_IFDIR, S_IFLNK, S_IFMT,
    S_IFREG, S_ISGID, S_ISUID, S_ISVTX, TCSADRAIN, TIOCGWINSZ, WEXITSTATUS, WIFEXITED, WIFSIGNALED,
    WNOHANG, WTERMSIG, W_OK, X_OK,
};

// D1/D46 job-control kill signals: `kill_tree`/`kill_single` grow a
// portable `Signal` parameter (`kill_cmd`'s `-SIG`/`-9`/`-CONT`,
// `fg_cmd`/`bg_cmd`'s `SIGCONT` resume). SIGKILL/SIGTERM/SIGINT/SIGHUP
// already admitted above (SIGKILL for the pre-`Signal` hard-kill default,
// SIGTERM/SIGINT/SIGHUP for the D6 received-signal source) — only the
// three job-control-specific identities are new here.
pub use libc::{SIGCONT, SIGQUIT, SIGSTOP};

// D10 wait-status completion: WUNTRACED/WCONTINUED opt a wait call into
// observing stop/continue transitions instead of only exit/signal
// termination; WIFSTOPPED/WSTOPSIG/WIFCONTINUED decode the resulting
// status word's stop/continue half (WIFEXITED/WIFSIGNALED/WEXITSTATUS/
// WTERMSIG, the exit/signal half, are already admitted above).
pub use libc::{WCONTINUED, WIFCONTINUED, WIFSTOPPED, WSTOPSIG, WUNTRACED};

// D1/D9 job-control terminal handoff: `tcsetpgrp(STDIN_FILENO, pgid)`
// gives/reclaims the controlling terminal's foreground process group.
// Sound only once SIGTTOU is ignored in the calling process (the D1
// precondition `JobControlTerminal::give_terminal` encodes rather than
// assumes) — SIGTTOU/SIG_IGN/SIG_DFL are admitted alongside it so that
// disposition can be set through this same curated surface rather than a
// second escape hatch. `signal`/`sighandler_t` (SIG_IGN's type) are
// already admitted above for the D6 signal source.
pub use libc::{tcsetpgrp, SIGTTOU, SIG_DFL, SIG_IGN};

// `test`'s file-mode predicates (faccessat slice's sibling, D11):
// S_ISUID/S_ISGID/S_ISVTX decode `st_mode` for `-u/-g/-k`; `st_uid`/
// `st_gid` (already reachable off `stat`, no separate admission needed)
// are `-O`/`-G`'s owner check. Used identically in both track-p
// configurations — plain POSIX mode-bit constants, not a syscall this
// crate's own libc-vs-raw-syscall split applies to.

// `Dir::set_unix_mode` (coreutils gap backlog #64 — `unix_mode`'s
// write-side companion): `fchmodat` is an ordinary POSIX libc wrapper,
// admitted only for the non-track-p arm — `rusty_libc`'s `fs` module has
// no `chmod`-family binding yet at the pinned rev, so the track-p arm is
// `Unsupported` rather than silently falling back to this admission
// (see `sys/fdio.rs::set_unix_mode`'s track-p comment).

// D11 Fs second wave: renameat2 has no libc wrapper on the glibc x86_64
// target at this repo's MSRV baseline (same situation as pidfd_open) —
// SYS_renameat2 + the raw syscall escape hatch. symlinkat/readlinkat
// (symlink slice) are ordinary POSIX libc wrapper functions, unlike
// renameat2 — no escape hatch needed for either configuration.
// faccessat (faccessat slice) is the same: an ordinary wrapper, called
// with flags=0 (real, not effective, uid/gid) in both configurations —
// rusty_libc's own `faccessat` has no flags parameter at all (only the
// bare-syscall real-id check), so this keeps the two configurations
// answering the identical question rather than glibc's userspace-only
// `AT_EACCESS` emulation quietly diverging from Track P.

// The terminal cluster (extraction map D9). `cfmakeraw` and `isatty` are
// libc *library* routines, not syscalls: cfmakeraw is the canonical
// raw-mode flag recipe, isatty is tcgetattr-in-disguise. `ioctl` is
// admitted solely for `TIOCGWINSZ` — the window-size query has no
// non-ioctl form.
pub use libc::{cfmakeraw, ioctl, isatty};

// Slice 2 (poll_readable/read_chunk/set_echo/is_raw, roadmap Phase 2).
// ICANON and ECHO are read from/written into the `c_lflag` field already
// reachable through `termios` above — admitted so `is_raw`'s live probe
// and `set_echo`'s bit flip don't need to re-derive cfmakeraw's mask.
pub use libc::{ECHO, ICANON};

// Net surface, TCP slice (RFC v2 R5+, D16). Not track-p-gated at all —
// unlike fs/process/terminal, sockets were never in rush's required
// surface (`DESIGN.md`'s ~25-syscall inventory has none), so rusty_libc
// has nothing here to route through; one implementation for both
// configurations, the same treatment `fsync` gets.
pub use libc::{
    accept4, bind, connect, getpeername, getsockname, listen, setsockopt, sockaddr, sockaddr_in,
    sockaddr_in6, sockaddr_storage, socket, socklen_t, AF_INET, AF_INET6, IPPROTO_TCP,
    SOCK_CLOEXEC, SOCK_STREAM, SOL_SOCKET, SOMAXCONN, SO_REUSEADDR, TCP_NODELAY,
};

// Net surface, Unix domain socket slice (RFC v2 R5+, D16 follow-on).
// `sockaddr_un`/`AF_UNIX` are the `AF_UNIX` counterpart to the TCP
// slice's `sockaddr_in{,6}`/`AF_INET{,6}` above; `chmod`/`mode_t` are
// admitted solely so `unix_listen` can narrow a freshly bound socket
// path to the agreed mode-0600 (owner-only) permissions the LocalAPI/
// agent consumers need — the same POSIX mode-bit territory `fs.rs`'s
// `UnixMode` already covers, but reached here via a plain path (no `Dir`
// borrow available at bind time).
pub use libc::{chmod, mode_t, sockaddr_un, AF_UNIX};

// Net surface, UDP datagram slice (RFC v2 R5+, D16, final slice) —
// rusty_tail's magicsock. `recvfrom`/`sendto` are UDP's connectionless
// counterpart to `read`/`write`: the peer address travels with every
// call instead of being fixed once at `connect`/`accept` time.
pub use libc::{recvfrom, sendto, SOCK_DGRAM};

// TcpStream::set_read_timeout (rusty_rdp convergence forcing consumer:
// its examples/connect.rs idles out a read loop once the server goes
// quiet via std::net::TcpStream::set_read_timeout — the one capability
// this trait was missing to accept platform's stream in place of std's).
// `SO_RCVTIMEO` takes a `struct timeval` on Linux (Windows' equivalent
// is a plain millisecond `DWORD` instead — see sys::net's doc comment).
pub use libc::{suseconds_t, time_t, timeval, SO_RCVTIMEO};

// Net surface, raw-fd + non-blocking escape hatch (rustils#41, rusty_tail's
// rusty_tokio hand-rolled async runtime — wants to register a socket with
// its own reactor rather than reimplement socket setup from scratch).
// Inherent-impl-only on the concrete Linux socket types, not part of the
// object-safe `platform::net` traits (see those types' own doc comments).
// `fcntl` has no dedicated admission elsewhere in this file (unlike
// `ioctl`, admitted solely for `TIOCGWINSZ`) — `F_GETFL`/`F_SETFL` read
// and write a socket's open-file-status flags, of which `O_NONBLOCK` is
// the one this escape hatch toggles.
pub use libc::{fcntl, F_GETFL, F_SETFL, O_NONBLOCK};

// Security surface, CSPRNG slice (RFC v2 R5+, D15, first slice) —
// rusty_rdp's five hand-rolled `/dev/urandom` reads. No libc *wrapper*
// function for `getrandom(2)` is admitted here (unlike `renameat2`,
// which at least has one at this repo's MSRV baseline) — the raw
// syscall via `SYS_getrandom`, same escape-hatch shape as `pidfd_open`.
pub use libc::SYS_getrandom;

// Security surface, sandbox policy slice (RFC v2 R5+, D15, Phase 6 item
// 3 — see docs/design-discussion-sandbox.md). Landlock has no libc
// *wrapper* functions at all (it's newer than pidfd_open/renameat2 ever
// were) — three raw syscalls via SYS_landlock_*, same escape-hatch shape
// as everything else in this file lacking one. `open`/`O_PATH` open each
// confined root as a directory-or-file handle `landlock_add_rule` takes
// as `parent_fd`. `prctl`/`PR_SET_NO_NEW_PRIVS`/`PR_SET_SECCOMP` gate
// both the Landlock ruleset (`landlock_restrict_self` requires
// `no_new_privs` or `CAP_SYS_ADMIN`) and the seccomp-BPF install
// (`block_inet_sockets`). `sock_filter`/`sock_fprog` are the BPF
// instruction/program types the seccomp-BPF filter is built from;
// `BPF_*`/`SECCOMP_*` are its opcode and verdict constants.
pub use libc::{
    open, prctl, sock_filter, sock_fprog, SYS_landlock_add_rule, SYS_landlock_create_ruleset,
    SYS_landlock_restrict_self, SYS_socket, AF_PACKET, BPF_ABS, BPF_JEQ, BPF_JMP, BPF_K, BPF_LD,
    BPF_RET, BPF_W, ENOSYS, EOPNOTSUPP, EPERM, O_PATH, PR_SET_NO_NEW_PRIVS, PR_SET_SECCOMP,
    SECCOMP_MODE_FILTER, SECCOMP_RET_ALLOW, SECCOMP_RET_DATA, SECCOMP_RET_ERRNO,
    SECCOMP_RET_KILL_PROCESS,
};

// Tun surface (RFC v2 R5+, D14) — rusty_tail's ts-tun, the single named
// consumer. `IFF_TUN`/`IFF_NO_PI`/`TUNSETIFF` are admitted here (unlike
// Landlock, this ioctl predates pidfd_open/renameat2 and libc does carry
// it); `SIOCSIFADDR`/`SIOCSIFNETMASK`/`SIOCSIFMTU`/`SIOCGIFFLAGS`/
// `SIOCSIFFLAGS` have no libc admission on this generic Linux target at
// all (only libc's android/l4re-specific paths carry them) — the same
// escape-hatch shape as Landlock's raw syscalls, just raw ioctl request
// numbers instead: stable, documented Linux kernel `ioctl(2)` request
// codes (`<linux/sockios.h>`), not expected to ever change. `AF_INET`/
// `SOCK_DGRAM`/`socket`/`ioctl`/`open`/`O_RDWR` are all already admitted
// above for the Net/Landlock slices.
pub use libc::{c_ulong, IFF_NO_PI, IFF_TUN, TUNSETIFF};
pub const SIOCSIFADDR: c_ulong = 0x8916;
pub const SIOCSIFNETMASK: c_ulong = 0x891c;
pub const SIOCSIFMTU: c_ulong = 0x8922;
pub const SIOCGIFFLAGS: c_ulong = 0x8913;
pub const SIOCSIFFLAGS: c_ulong = 0x8914;

// Fs surface, `File::try_clone` (D5, rustils#51 — the `2>&1`/`&> file`
// shell-redirect shape `nexus-rush/src/exec.rs::build_stage` needs).
// `F_DUPFD_CLOEXEC` duplicates a fd to the lowest available number with
// `CLOEXEC` set atomically on the new fd — the same fd-family `fcntl`
// already admitted above for the Net rustils#41 escape hatch, one more
// `cmd` value on it.
pub use libc::F_DUPFD_CLOEXEC;

// uid/gid → display-name resolution (coreutils gap backlog #65, `ls -l`
// donor material): the NSS-backed reentrant lookups, not the classic
// non-reentrant `getpwuid`/`getgrgid` (a shared static buffer across
// every caller in the process — exactly the kind of hazard this crate's
// own `LAST_ERRNO` thread-local commentary elsewhere already flags).
pub use libc::{getgrgid_r, getpwuid_r, group, passwd, ERANGE};

// D-Bus transport (RFC v2 R5+, D15, Phase 6 item 2, rustils#77): `getuid`
// is SASL `EXTERNAL`'s own credential — the ASCII decimal UID, hex-
// encoded, is the entire authentication payload for a local unix-socket
// peer (the kernel already knows who's on the other end of the socket;
// SASL EXTERNAL just asks the peer to state it, not prove it out-of-band
// the way a password mechanism would).
pub use libc::getuid;

// PTY surface (RFC v2 R5+, D13, convergence roadmap Phase 7, rustils#82).
// `posix_openpt`/`grantpt`/`unlockpt`/`ptsname_r` are the standard POSIX
// pty-pair-opening sequence (`ptsname_r`, not the non-reentrant
// `ptsname`, for the same shared-static-buffer reason `getpwuid_r` was
// chosen over `getpwuid` above); `size_t` is `ptsname_r`'s buffer-length
// parameter type, not otherwise admitted. `O_NOCTTY` guards the *master*
// open only — the slave is deliberately opened without it (see
// `sys::pty`'s own doc comment: that's what lets a session leader with no
// controlling terminal acquire one automatically). `TIOCSWINSZ` is
// `TIOCGWINSZ`'s write-side sibling (already admitted above), for
// `PtyMaster::resize`. `POSIX_SPAWN_SETSID` (a glibc extension since
// 2.24) is the `posix_spawn`-native substitute for `fork`+`TIOCSCTTY`
// this crate uses instead — see `docs/design-discussion-pty.md`'s "The
// `posix_spawn` substitute for `fork`+`TIOCSCTTY`" section for why: raw
// `fork` stays parked behind its own separate roadmap decision, not
// reopened here.
pub use libc::{
    grantpt, posix_openpt, ptsname_r, size_t, unlockpt, O_NOCTTY, POSIX_SPAWN_SETSID, TIOCSWINSZ,
};

// `platform::fs::AnonymousFile` (D11, landed independently per
// `docs/decision-request-fork-execve.md`'s option 3). `memfd_create` has
// carried a real libc wrapper since long before this repo's MSRV floor
// (unlike `renameat2`/`getrandom`/Landlock above) — an ordinary admission,
// not an escape hatch, and not track-p-gated (the same "one implementation
// for both configurations" treatment `fsync`/the Net surface get): no
// consumer has ever asked rusty_libc for a raw-syscall form of this one.
// `MFD_CLOEXEC` matches every fd this crate creates (RFC v2's
// CLOEXEC-by-default discipline — `openat` above ORs it into every open
// unconditionally).
pub use libc::{memfd_create, MFD_CLOEXEC};
// `lseek`/`SEEK_SET`: test-only, for `memfd_create`'s own conformance
// tests (rewind after a raw `write` to read back what was just written).
pub use libc::{lseek, SEEK_SET};

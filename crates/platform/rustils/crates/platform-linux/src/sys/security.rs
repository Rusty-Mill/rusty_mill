//! CSPRNG (RFC v2 R5+, D15, first Security surface slice): the raw
//! `getrandom(2)` syscall, not `/dev/urandom` as a file — no `fd` for a
//! caller under a filesystem sandbox to have denied (see
//! `platform::security`'s module doc comment).
//!
//! Sandbox policy (D15, Phase 6 item 3): raw Landlock syscalls for
//! filesystem confinement, raw seccomp-BPF for network-socket
//! confinement. Mirrors nexus's `os_sandbox.rs` shape exactly (see
//! `docs/design-discussion-sandbox.md`) — two independently-degradable
//! calls, not one, because that's the shape nexus's own implementation
//! proved necessary.

#![allow(unsafe_code)]

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::security::SandboxStatus;

use crate::ffi::libc_surface as c;

fn errno_err(op: &'static str, code: i32) -> PlatformError {
    let kind = match code {
        libc::ENOSYS => ErrorKind::Unsupported,
        _ => ErrorKind::Other,
    };
    PlatformError::new(kind, OsCode::Errno(code), op)
}

/// Fill `buf` with `getrandom(buf, buf.len(), 0)` — flags `0` blocks
/// until the CRNG is seeded (practically instantaneous after early
/// boot) and draws from the same pool `/dev/urandom` does. Retries on
/// `EINTR` and on the short reads `getrandom` can return for requests
/// over 256 bytes, since the syscall (unlike `read(2)` on a regular
/// file) makes no promise to fill the whole buffer in one call.
pub fn fill_random(buf: &mut [u8]) -> Result<()> {
    let mut filled = 0;
    while filled < buf.len() {
        // SAFETY: `buf[filled..]` is a valid, writable region of the
        // stated length for the duration of the call; `getrandom` writes
        // no more bytes than the length given and takes no other
        // pointer argument.
        let r = unsafe {
            c::syscall(
                c::SYS_getrandom,
                buf[filled..].as_mut_ptr(),
                buf.len() - filled,
                0u32,
            )
        };
        if r < 0 {
            let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if code == libc::EINTR {
                continue;
            }
            return Err(errno_err("getrandom", code));
        }
        filled += r as usize;
    }
    Ok(())
}

// --- Landlock filesystem confinement -------------------------------------

/// ABI v1 `struct landlock_ruleset_attr` (`linux/landlock.h`) — exactly the
/// one field ABI v1 defines. Later ABI versions only ever *append* fields;
/// passing `size_of::<Self>()` (8 bytes) to `landlock_create_ruleset`
/// requests ABI v1 semantics regardless of what the running kernel's own
/// max ABI actually is — the kernel uses the caller's `size` to know which
/// subset was requested, so this stays correct even on a much newer
/// kernel.
#[repr(C)]
struct LandlockRulesetAttr {
    handled_access_fs: u64,
}

/// `struct landlock_path_beneath_attr` (`linux/landlock.h`) — packed, no
/// padding between the two fields, matching the kernel's own layout.
#[repr(C, packed)]
struct LandlockPathBeneathAttr {
    allowed_access: u64,
    parent_fd: i32,
}

const LANDLOCK_RULE_PATH_BENEATH: i32 = 1;

const ACCESS_FS_EXECUTE: u64 = 1 << 0;
const ACCESS_FS_WRITE_FILE: u64 = 1 << 1;
const ACCESS_FS_READ_FILE: u64 = 1 << 2;
const ACCESS_FS_READ_DIR: u64 = 1 << 3;
const ACCESS_FS_REMOVE_DIR: u64 = 1 << 4;
const ACCESS_FS_REMOVE_FILE: u64 = 1 << 5;
const ACCESS_FS_MAKE_CHAR: u64 = 1 << 6;
const ACCESS_FS_MAKE_DIR: u64 = 1 << 7;
const ACCESS_FS_MAKE_REG: u64 = 1 << 8;
const ACCESS_FS_MAKE_SOCK: u64 = 1 << 9;
const ACCESS_FS_MAKE_FIFO: u64 = 1 << 10;
const ACCESS_FS_MAKE_BLOCK: u64 = 1 << 11;
const ACCESS_FS_MAKE_SYM: u64 = 1 << 12;

/// Every access right ABI v1 defines — granted on `writable_roots`, and
/// the `handled_access_fs` mask the ruleset itself is created with (any
/// access kind in this set is denied everywhere except where a rule
/// explicitly grants it).
const ABI_V1_ALL_ACCESS: u64 = ACCESS_FS_EXECUTE
    | ACCESS_FS_WRITE_FILE
    | ACCESS_FS_READ_FILE
    | ACCESS_FS_READ_DIR
    | ACCESS_FS_REMOVE_DIR
    | ACCESS_FS_REMOVE_FILE
    | ACCESS_FS_MAKE_CHAR
    | ACCESS_FS_MAKE_DIR
    | ACCESS_FS_MAKE_REG
    | ACCESS_FS_MAKE_SOCK
    | ACCESS_FS_MAKE_FIFO
    | ACCESS_FS_MAKE_BLOCK
    | ACCESS_FS_MAKE_SYM;

/// Granted on `readable_roots`: read a file's contents, list a
/// directory's entries, execute a file. No write/create/delete rights.
const READ_ONLY_ACCESS: u64 = ACCESS_FS_EXECUTE | ACCESS_FS_READ_FILE | ACCESS_FS_READ_DIR;

fn open_path_fd(path: &Path) -> Result<i32> {
    let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        PlatformError::new(ErrorKind::InvalidInput, OsCode::None, "confine_filesystem")
            .with_path(path)
    })?;
    // SAFETY: `c_path` is a valid NUL-terminated string outliving this
    // call. `O_PATH` opens a lightweight handle usable only for
    // permission-check-free path resolution — exactly what
    // `landlock_add_rule`'s `parent_fd` needs, and it needs no `mode`
    // argument since `O_CREAT` is never set.
    let fd = unsafe { c::open(c_path.as_ptr(), c::O_PATH | c::O_CLOEXEC) };
    if fd < 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return Err(errno_err("open", code).with_path(path));
    }
    Ok(fd)
}

/// `prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0)` — required before
/// `landlock_restrict_self` (which otherwise needs `CAP_SYS_ADMIN`) and
/// before installing a seccomp filter as a non-root, non-privileged
/// caller. Idempotent: setting it again once already set is a no-op
/// success, so both `confine_filesystem` and `block_inet_sockets` can
/// call this independently without caring which ran first.
fn set_no_new_privs() -> Result<()> {
    // SAFETY: this `prctl` option takes no pointer arguments; the
    // trailing zeros are its documented unused-argument convention.
    let r = unsafe { c::prctl(c::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if r < 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return Err(errno_err("prctl(PR_SET_NO_NEW_PRIVS)", code));
    }
    Ok(())
}

/// `landlock_create_ruleset(&attr, size_of(attr), 0)`. `Ok(None)` means
/// the kernel has no usable Landlock (missing entirely, pre-5.13, or
/// disabled via boot parameter) — the caller's cue to report
/// `SandboxStatus::NotEnforced` rather than erroring, matching nexus's
/// own degrade-not-fail design.
fn create_ruleset() -> Result<Option<i32>> {
    let attr = LandlockRulesetAttr {
        handled_access_fs: ABI_V1_ALL_ACCESS,
    };
    // SAFETY: `attr` is a valid, correctly-sized struct for the ABI v1
    // request its size encodes; `landlock_create_ruleset` reads it once
    // and takes no other pointer argument.
    let fd = unsafe {
        c::syscall(
            c::SYS_landlock_create_ruleset,
            &attr as *const LandlockRulesetAttr,
            std::mem::size_of::<LandlockRulesetAttr>(),
            0u32,
        )
    };
    if fd < 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if code == c::ENOSYS || code == c::EOPNOTSUPP {
            return Ok(None);
        }
        return Err(errno_err("landlock_create_ruleset", code));
    }
    Ok(Some(fd as i32))
}

fn add_rule(ruleset_fd: i32, path: &Path, allowed_access: u64) -> Result<()> {
    let parent_fd = open_path_fd(path)?;
    let rule_attr = LandlockPathBeneathAttr {
        allowed_access,
        parent_fd,
    };
    // SAFETY: `rule_attr` is a valid, correctly-packed struct; `parent_fd`
    // is a live handle just opened above; `landlock_add_rule` reads the
    // struct once and takes no other pointer argument.
    let r = unsafe {
        c::syscall(
            c::SYS_landlock_add_rule,
            ruleset_fd,
            LANDLOCK_RULE_PATH_BENEATH,
            &rule_attr as *const LandlockPathBeneathAttr,
            0u32,
        )
    };
    // Capture any failure before `close`, which can itself set errno.
    let code = if r < 0 {
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
    } else {
        0
    };
    // SAFETY: `parent_fd` is a valid fd owned exclusively by this function.
    unsafe { c::close(parent_fd) };
    if r < 0 {
        return Err(errno_err("landlock_add_rule", code).with_path(path));
    }
    Ok(())
}

/// Deny all filesystem access except read+execute under `readable_roots`
/// and full access under `writable_roots`, for the calling thread only —
/// see `platform::security::Sandbox`'s trait doc comment for the
/// single-threaded-caller contract this relies on.
pub fn confine_filesystem(
    readable_roots: &[&Path],
    writable_roots: &[&Path],
) -> Result<SandboxStatus> {
    let ruleset_fd = match create_ruleset()? {
        Some(fd) => fd,
        None => return Ok(SandboxStatus::NotEnforced),
    };

    let result = (|| -> Result<()> {
        for root in readable_roots {
            add_rule(ruleset_fd, root, READ_ONLY_ACCESS)?;
        }
        for root in writable_roots {
            add_rule(ruleset_fd, root, ABI_V1_ALL_ACCESS)?;
        }
        set_no_new_privs()?;
        // SAFETY: `ruleset_fd` is a valid, live ruleset handle from
        // `create_ruleset` above, with every rule added above already
        // applied to it.
        let r = unsafe { c::syscall(c::SYS_landlock_restrict_self, ruleset_fd, 0u32) };
        if r < 0 {
            let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            return Err(errno_err("landlock_restrict_self", code));
        }
        Ok(())
    })();

    // SAFETY: `ruleset_fd` is a valid fd owned exclusively by this
    // function; closing it after `landlock_restrict_self` has consumed it
    // is safe and matches nexus's own cleanup.
    unsafe { c::close(ruleset_fd) };
    result?;
    Ok(SandboxStatus::Enforced)
}

// --- seccomp-BPF network-socket confinement -------------------------------

// `struct seccomp_data` (`linux/seccomp.h`) field byte offsets. Hand-
// computed rather than `std::mem::offset_of!` (stabilized in Rust 1.77,
// above this workspace's 1.75 MSRV): `nr: c_int` at 0, `arch: __u32` at
// 4, `instruction_pointer: __u64` at 8 (8-byte aligned), `args: [__u64; 6]`
// at 16. This is a stable kernel UAPI layout, not implementation-defined.
const SECCOMP_DATA_NR_OFFSET: u32 = 0;
const SECCOMP_DATA_ARCH_OFFSET: u32 = 4;
const SECCOMP_DATA_ARG0_OFFSET: u32 = 16;

/// `AUDIT_ARCH_X86_64` (`linux/audit.h`): `EM_X86_64 (62) | __AUDIT_ARCH_64BIT
/// (0x8000_0000) | __AUDIT_ARCH_LE (0x4000_0000)`. The filter checks this
/// first and kills the process on a mismatch — the standard seccomp-BPF
/// defense against a 32-bit syscall-number reinterpretation bypassing the
/// filter (this repo currently only targets x86_64; see
/// `block_inet_sockets`'s non-x86_64 stub below).
#[cfg(target_arch = "x86_64")]
const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;

#[cfg(target_arch = "x86_64")]
fn bpf_stmt(code: u32, k: u32) -> c::sock_filter {
    c::sock_filter {
        code: code as u16,
        jt: 0,
        jf: 0,
        k,
    }
}

#[cfg(target_arch = "x86_64")]
fn bpf_jump(code: u32, k: u32, jt: u8, jf: u8) -> c::sock_filter {
    c::sock_filter {
        code: code as u16,
        jt,
        jf,
        k,
    }
}

/// The filter: kill the process on an unexpected architecture (defensive
/// only — this function is `x86_64`-gated); allow every syscall except
/// `socket()`; for `socket()`, deny with `EPERM` when the requested
/// domain is `AF_INET`/`AF_INET6`/`AF_PACKET` and allow everything else
/// (`AF_UNIX` included) — mirroring nexus's own narrow "no raw internet
/// sockets" scope exactly, not a general syscall allowlist.
#[cfg(target_arch = "x86_64")]
fn build_seccomp_program() -> [c::sock_filter; 11] {
    let ld_w_abs = c::BPF_LD | c::BPF_W | c::BPF_ABS;
    let jmp_jeq_k = c::BPF_JMP | c::BPF_JEQ | c::BPF_K;
    let ret_k = c::BPF_RET | c::BPF_K;

    [
        bpf_stmt(ld_w_abs, SECCOMP_DATA_ARCH_OFFSET),     // 0
        bpf_jump(jmp_jeq_k, AUDIT_ARCH_X86_64, 0, 8),     // 1: mismatch -> 10 (kill)
        bpf_stmt(ld_w_abs, SECCOMP_DATA_NR_OFFSET),       // 2
        bpf_jump(jmp_jeq_k, c::SYS_socket as u32, 0, 4),  // 3: not socket() -> 8 (allow)
        bpf_stmt(ld_w_abs, SECCOMP_DATA_ARG0_OFFSET),     // 4: load `domain` arg
        bpf_jump(jmp_jeq_k, libc::AF_INET as u32, 3, 0),  // 5: AF_INET -> 9 (errno)
        bpf_jump(jmp_jeq_k, libc::AF_INET6 as u32, 2, 0), // 6: AF_INET6 -> 9
        bpf_jump(jmp_jeq_k, c::AF_PACKET as u32, 1, 0),   // 7: AF_PACKET -> 9
        bpf_stmt(ret_k, c::SECCOMP_RET_ALLOW),            // 8
        bpf_stmt(
            ret_k,
            c::SECCOMP_RET_ERRNO | (c::EPERM as u32 & c::SECCOMP_RET_DATA),
        ), // 9
        bpf_stmt(ret_k, c::SECCOMP_RET_KILL_PROCESS),     // 10
    ]
}

/// Deny opening new `AF_INET`/`AF_INET6`/`AF_PACKET` sockets from the
/// calling thread onward, via `prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER,
/// ...)`. Currently `x86_64`-only (see `AUDIT_ARCH_X86_64`'s doc
/// comment) — every other architecture reports `Unsupported`.
#[cfg(target_arch = "x86_64")]
pub fn block_inet_sockets() -> Result<SandboxStatus> {
    set_no_new_privs()?;
    let program = build_seccomp_program();
    let fprog = c::sock_fprog {
        len: program.len() as std::ffi::c_ushort,
        filter: program.as_ptr().cast_mut(),
    };
    // SAFETY: `fprog.filter` points at `program`, which outlives this
    // call; `prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, ...)` reads the
    // program once during the call and retains no pointer afterward.
    let r = unsafe {
        c::prctl(
            c::PR_SET_SECCOMP,
            c::SECCOMP_MODE_FILTER,
            &fprog as *const c::sock_fprog,
            0,
            0,
        )
    };
    if r < 0 {
        let code = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if code == c::ENOSYS {
            return Ok(SandboxStatus::NotEnforced);
        }
        return Err(errno_err("prctl(PR_SET_SECCOMP)", code));
    }
    Ok(SandboxStatus::Enforced)
}

#[cfg(not(target_arch = "x86_64"))]
pub fn block_inet_sockets() -> Result<SandboxStatus> {
    Ok(SandboxStatus::Unsupported)
}

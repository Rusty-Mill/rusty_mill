//! Linux-only R2 mount-confinement verification (Rusty-Mill fs slice):
//! `sys::fdio::openat_r2`'s `RESOLVE_NO_XDEV` request actually rejects a
//! path resolution that would cross a mount boundary, not just that the
//! flag is requested in code. This is the exact gap
//! `docs/divergences.md` #013 and the Rusty-Mill `rustils-comparison.md`
//! record (`RT-002`, TRIAL-0002) disclosed as untested — "a bind-mount
//! fixture needing elevated privilege this test harness cannot assume
//! in CI." `#![cfg(target_os = "linux")]`: integration test files don't
//! inherit the library crate's own platform gate — see `tun_parity.rs`'s
//! identical header note.
#![cfg(target_os = "linux")]
#![allow(unsafe_code)]

use std::ffi::{CString, OsStr};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use platform::error::ErrorKind;
use platform::fs::Dir;

/// Mount a fresh `tmpfs` at `target` — definitionally a distinct
/// filesystem/device from whatever `target`'s parent lives on
/// (unlike a same-device bind mount, where crossing semantics are a
/// documented but separate subtlety this test doesn't need to take a
/// position on). Requires `CAP_SYS_ADMIN`; returns the raw errno on
/// failure so the caller can skip gracefully rather than panic —
/// the same honest-skip discipline `tun_parity.rs`'s `tun_or_skip!`
/// established for its own privilege gap.
fn mount_tmpfs(target: &Path) -> Result<(), i32> {
    let target_c = CString::new(target.as_os_str().as_bytes()).expect("no interior NUL");
    let fstype_c = CString::new("tmpfs").expect("no interior NUL");
    // SAFETY: `target_c`/`fstype_c` are valid, NUL-terminated, live C
    // strings for the duration of this call; `data` is null (no mount
    // options); this is exactly `mount(2)`'s documented signature for a
    // virtual filesystem with no block-device source.
    let rc = unsafe {
        libc::mount(
            fstype_c.as_ptr(),
            target_c.as_ptr(),
            fstype_c.as_ptr(),
            0,
            std::ptr::null(),
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        // SAFETY: reading the calling thread's own errno immediately
        // after the failing libc call that set it, before anything else
        // in this thread can overwrite it.
        Err(unsafe { *libc::__errno_location() })
    }
}

/// Best-effort lazy unmount (`MNT_DETACH`) — robust even if something
/// still has `target` open, unlike a plain `umount2` with no flags.
fn unmount(target: &Path) {
    let target_c = match CString::new(target.as_os_str().as_bytes()) {
        Ok(c) => c,
        Err(_) => return,
    };
    // SAFETY: `target_c` is a valid, NUL-terminated, live C string for
    // the duration of this call; best-effort cleanup, result ignored.
    unsafe {
        libc::umount2(target_c.as_ptr(), libc::MNT_DETACH);
    }
}

/// Skip (not fail) when `CAP_SYS_ADMIN` is unavailable, mirroring
/// `tun_parity.rs`'s `tun_or_skip!` for its own privilege gap.
macro_rules! mount_or_skip {
    ($tmp:expr, $mnt:expr) => {
        if let Err(errno) = mount_tmpfs($mnt) {
            eprintln!(
                "skipping: mount(2) unavailable in this environment (errno {errno}, needs CAP_SYS_ADMIN)"
            );
            std::fs::remove_dir_all($tmp).ok();
            return;
        }
    };
}

#[test]
fn linux_open_dir_rejects_a_mount_crossing_in_an_intermediate_component() {
    if !platform_linux::sys::fdio::openat2_supported() {
        eprintln!("skipping: openat2 unsupported on this kernel (pre-5.6)");
        return;
    }

    let tmp = std::env::temp_dir().join(format!("rustils-r2-mount-open-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let mnt = tmp.join("mnt");
    std::fs::create_dir(&mnt).expect("mkdir mnt");

    mount_or_skip!(&tmp, &mnt);

    // "leaf" lives inside the freshly mounted tmpfs, created directly
    // (fixture setup, not the behavior under test) so `open_dir` has
    // something real to resolve on the far side of the boundary.
    std::fs::create_dir(mnt.join("leaf")).expect("mkdir mnt/leaf");

    let root = platform_linux::LinuxDir::open_ambient(&tmp).expect("open ambient");
    let e = root
        .open_dir(OsStr::new("mnt/leaf"))
        .map(|_| ()) // `Box<dyn Dir>` isn't `Debug`; `expect_err` needs the `Ok` side to be
        .expect_err("a mount-point crossing must now be rejected under R2");
    assert_eq!(e.kind, ErrorKind::CrossesDevices);

    // Sanity: an R1 op (metadata, plain `openat`/`fstatat`, no
    // `RESOLVE_NO_XDEV`) still resolves straight through the same
    // boundary — this is R2's deliberate containment gain on
    // `open_dir`/`create_dir`, not a generic breakage of the mount.
    root.metadata(OsStr::new("mnt/leaf"))
        .expect("an R1 op must still resolve across the same mount boundary");

    unmount(&mnt);
    std::fs::remove_dir_all(&tmp).ok();
}

/// R2 mount-confinement for `create_dir`'s multi-component
/// parent-resolution path — same crossing, `create_dir`'s own call
/// site rather than `open_dir`'s.
#[test]
fn linux_create_dir_rejects_a_mount_crossing_in_an_intermediate_component() {
    if !platform_linux::sys::fdio::openat2_supported() {
        eprintln!("skipping: openat2 unsupported on this kernel (pre-5.6)");
        return;
    }

    let tmp = std::env::temp_dir().join(format!("rustils-r2-mount-create-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("mk tempdir");
    let mnt = tmp.join("mnt");
    std::fs::create_dir(&mnt).expect("mkdir mnt");

    mount_or_skip!(&tmp, &mnt);

    let root = platform_linux::LinuxDir::open_ambient(&tmp).expect("open ambient");
    let e = root
        .create_dir(OsStr::new("mnt/newdir"))
        .expect_err("a mount-point crossing must now be rejected under R2, not silently create on the other side");
    assert_eq!(e.kind, ErrorKind::CrossesDevices);

    // Confirm the rejection is real, not a partial/silent create on the
    // far side of the boundary before the error was returned.
    let not_created = root.metadata(OsStr::new("mnt/newdir")).expect_err(
        "create_dir must not have silently created anything on the other side of the boundary",
    );
    assert_eq!(not_created.kind, ErrorKind::NotFound);

    unmount(&mnt);
    std::fs::remove_dir_all(&tmp).ok();
}

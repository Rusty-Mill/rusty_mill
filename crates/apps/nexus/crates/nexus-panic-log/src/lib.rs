//! Local panic logging for Nexus binaries.
//!
//! Installs a [`std::panic::set_hook`] that appends a structured entry to
//! `~/.nexus-shell/logs/panic.log` on every panic, then chains to the
//! previously-installed hook so stderr output is preserved. No network, no
//! opt-in UI — Nexus is a personal tool and panics should survive a closed
//! terminal.
//!
//! Rotation policy: if the log file exceeds 1 MiB before a write, it is
//! renamed to `panic.log.1` (overwriting any prior `.1`). Two-file ceiling.
//!
//! Failures inside the hook are swallowed intentionally — a panic-in-hook
//! loop is worse than a missed log line.

use std::backtrace::Backtrace;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Once;

/// 1 MiB — rotation threshold.
const MAX_LOG_BYTES: u64 = 1024 * 1024;

/// Open (creating if needed) `path` for append, with permissions
/// restricted to the file owner. A panic message and its backtrace can
/// contain sensitive data captured in local variables/arguments; on a
/// multi-user system a default-mode log file is world-readable, so both
/// the initial log file and its rotated copy must be locked down at
/// creation/rotation time rather than relying on a later `chmod`.
fn open_restricted(path: &std::path::Path) -> std::io::Result<fs::File> {
    let mut opts = OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let file = opts.open(path)?;
    restrict_permissions(path)?;
    Ok(file)
}

/// Best-effort-on-every-call permission lockdown for `path`. On Unix this
/// re-asserts owner-only `0o600` (covers files that predate this fix and
/// files that survive a rename during rotation). On Windows this replaces
/// the file's DACL with a single owner-only, full-control ACE via the
/// Win32 security APIs — there is no `OpenOptions` equivalent of Unix's
/// `mode()` on Windows, so the ACL is applied immediately after creation.
#[cfg(unix)]
fn restrict_permissions(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(windows)]
fn restrict_permissions(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        SetFileSecurityW, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    };

    // "D:P(A;;FA;;;OW)" — a protected DACL containing exactly one ACE:
    // Allow, Full Access, to the Owner Rights well-known SID. This wholly
    // replaces the file's current DACL (no inherited entries survive),
    // so no other account — including other local users or "Everyone" —
    // retains access after this call.
    let sddl: Vec<u16> = "D:P(A;;FA;;;OW)\0".encode_utf16().collect();
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: `sddl` is a valid NUL-terminated UTF-16 buffer for the
    // duration of this call; `sd` is an out-param the OS populates with a
    // heap allocation that we free below via `LocalFree`.
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            std::ptr::null_mut(),
        )
    };
    if converted == 0 || sd.is_null() {
        return Err(std::io::Error::last_os_error());
    }

    let wide_path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `wide_path` is a valid NUL-terminated UTF-16 buffer; `sd` was
    // just validated non-null above and remains valid until freed below.
    let applied = unsafe { SetFileSecurityW(wide_path.as_ptr(), DACL_SECURITY_INFORMATION, sd) };
    let result = if applied == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    };

    // SAFETY: `sd` was allocated by
    // `ConvertStringSecurityDescriptorToSecurityDescriptorW` and must be
    // freed with `LocalFree` per its documented contract.
    unsafe {
        LocalFree(sd);
    }

    result
}

#[cfg(not(any(unix, windows)))]
fn restrict_permissions(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

static INSTALL_ONCE: Once = Once::new();

/// Install the panic hook for `binary_name`.
///
/// Safe to call multiple times; only the first call takes effect per process.
/// Call this as the first statement of `main()`, before any code that might
/// panic (argument parsing, tracing setup, etc.).
pub fn install(binary_name: &'static str) {
    INSTALL_ONCE.call_once(|| {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            // Best-effort write; never propagate an error.
            let _ = write_entry(binary_name, panic_info);
            // Always chain to the default hook so stderr still shows the panic.
            default_hook(panic_info);
        }));
    });
}

/// Resolve `~/.nexus-shell/logs/panic.log`. Matches the `dirs::home_dir`
/// convention used elsewhere in the Nexus codebase (see
/// `shell/src-tauri/src/lib.rs`).
fn log_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".nexus-shell").join("logs").join("panic.log"))
}

/// Rotate `panic.log` -> `panic.log.1` if the current file exceeds
/// [`MAX_LOG_BYTES`]. Errors are swallowed (caller continues regardless).
fn rotate_if_needed(path: &PathBuf) {
    let Ok(meta) = fs::metadata(path) else {
        return;
    };
    if meta.len() <= MAX_LOG_BYTES {
        return;
    }
    let rotated = path.with_extension("log.1");
    // `rename` overwrites the destination on Unix; on Windows we remove first.
    #[cfg(windows)]
    {
        let _ = fs::remove_file(&rotated);
    }
    if fs::rename(path, &rotated).is_ok() {
        // The rotated file may predate this fix (created with default OS
        // permissions); re-assert owner-only access on the copy too.
        let _ = restrict_permissions(&rotated);
    }
}

/// Append one entry to the panic log. Returns `Err` on any I/O failure, but
/// the caller (the panic hook) ignores the result.
fn write_entry(
    binary_name: &str,
    panic_info: &std::panic::PanicHookInfo<'_>,
) -> std::io::Result<()> {
    let path = log_path()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home directory"))?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    rotate_if_needed(&path);

    let timestamp = chrono::Utc::now().to_rfc3339();
    let location = panic_info
        .location()
        .map(|l| format!("{}:{}", l.file(), l.line()))
        .unwrap_or_else(|| "<unknown>".to_string());
    let message = panic_message(panic_info);
    let backtrace = Backtrace::force_capture();

    let mut file = open_restricted(&path)?;

    writeln!(file, "---")?;
    writeln!(file, "timestamp: {}", timestamp)?;
    writeln!(file, "binary:    {}", binary_name)?;
    writeln!(file, "location:  {}", location)?;
    writeln!(file, "message:   {}", message)?;
    writeln!(file, "backtrace:")?;
    writeln!(file, "{}", backtrace)?;
    Ok(())
}

/// Extract the panic payload as a string (handles `&str` and `String`).
fn panic_message(panic_info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = panic_info.payload();
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: write_entry produces a non-empty log file with the
    /// expected header fields. We invoke the hook machinery directly via
    /// `catch_unwind` + a custom hook that calls `write_entry_to` on a
    /// tempdir path.
    #[test]
    fn writes_entry_on_panic() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let log_path = tmp.path().join("panic.log");

        // Install a hook that routes into a local write_entry override.
        let log_path_clone = log_path.clone();
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = write_entry_to(&log_path_clone, "nexus-test", info);
        }));

        let result = std::panic::catch_unwind(|| {
            panic!("smoke-test panic");
        });

        // Restore prior hook before asserting so test output stays clean.
        let _ = std::panic::take_hook();
        std::panic::set_hook(prev);

        assert!(result.is_err(), "panic should have occurred");
        let contents = fs::read_to_string(&log_path).expect("log file written");
        assert!(
            contents.contains("binary:    nexus-test"),
            "contents: {contents}"
        );
        assert!(
            contents.contains("smoke-test panic"),
            "contents: {contents}"
        );
        assert!(contents.contains("timestamp:"), "contents: {contents}");
    }

    /// Test-only variant of `write_entry` that writes to a caller-supplied
    /// path instead of the real `~/.nexus-shell/logs/panic.log`.
    fn write_entry_to(
        path: &PathBuf,
        binary_name: &str,
        panic_info: &std::panic::PanicHookInfo<'_>,
    ) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        rotate_if_needed(path);
        let timestamp = chrono::Utc::now().to_rfc3339();
        let location = panic_info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "<unknown>".to_string());
        let message = panic_message(panic_info);
        let backtrace = Backtrace::force_capture();

        let mut file = open_restricted(path)?;
        writeln!(file, "---")?;
        writeln!(file, "timestamp: {}", timestamp)?;
        writeln!(file, "binary:    {}", binary_name)?;
        writeln!(file, "location:  {}", location)?;
        writeln!(file, "message:   {}", message)?;
        writeln!(file, "backtrace:")?;
        writeln!(file, "{}", backtrace)?;
        Ok(())
    }

    #[test]
    fn rotation_renames_oversized_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let log = tmp.path().join("panic.log");
        // Create a >1MiB file.
        fs::write(&log, vec![b'x'; (MAX_LOG_BYTES + 1) as usize]).unwrap();
        rotate_if_needed(&log);
        assert!(!log.exists(), "original log should have been rotated away");
        assert!(log.with_extension("log.1").exists(), ".log.1 should exist");
    }

    /// Regression test: a freshly created panic log file must have
    /// owner-only permissions, not the OS default (which is
    /// world-readable on many multi-user Unix systems). Panic messages
    /// and backtraces can carry sensitive data captured in local
    /// variables/arguments, so a permissive mode would leak it to any
    /// other local user.
    #[test]
    #[cfg(unix)]
    fn log_file_created_with_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().expect("tempdir");
        let log_path = tmp.path().join("panic.log");
        let _file = open_restricted(&log_path).expect("open_restricted");

        let mode = fs::metadata(&log_path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o600,
            "panic log must be owner-only (0o600), got {mode:o}"
        );
    }

    /// Windows counterpart: the file's DACL must contain no ACE granting
    /// access to anyone other than the owner (in particular, none for the
    /// well-known "Everyone"/"Authenticated Users"/"Users" SIDs) after
    /// `restrict_permissions` runs — proving the wide-open default ACL
    /// inherited from the parent directory was actually replaced.
    #[test]
    #[cfg(windows)]
    fn log_file_created_with_owner_only_permissions() {
        use std::os::windows::ffi::OsStrExt;

        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
        use windows_sys::Win32::Security::{
            AclSizeInformation, GetAce, GetAclInformation, ACCESS_ALLOWED_ACE, ACL,
            ACL_SIZE_INFORMATION, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        };
        use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;

        let tmp = tempfile::tempdir().expect("tempdir");
        let log_path = tmp.path().join("panic.log");
        let _file = open_restricted(&log_path).expect("open_restricted");

        let wide_path: Vec<u16> = log_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `wide_path` is NUL-terminated UTF-16 for a path that
        // exists (created just above); the out-params receive
        // OS-allocated data freed via `LocalFree` below.
        let status = unsafe {
            GetNamedSecurityInfoW(
                wide_path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut dacl,
                std::ptr::null_mut(),
                &mut sd,
            )
        };
        assert_eq!(status, 0, "GetNamedSecurityInfoW failed: {status}");
        assert!(!dacl.is_null(), "expected a DACL to be present");

        let mut size_info = ACL_SIZE_INFORMATION {
            AceCount: 0,
            AclBytesInUse: 0,
            AclBytesFree: 0,
        };
        // SAFETY: `dacl` is non-null and valid per the successful call
        // above; `size_info` is correctly sized for `AclSizeInformation`.
        let ok = unsafe {
            GetAclInformation(
                dacl,
                &mut size_info as *mut _ as *mut core::ffi::c_void,
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        };
        assert_ne!(ok, 0, "GetAclInformation failed");
        assert_eq!(
            size_info.AceCount, 1,
            "expected exactly one ACE (owner-only), got {}",
            size_info.AceCount
        );

        // SAFETY: index 0 is valid since `AceCount == 1` was just asserted.
        let mut ace_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let ok = unsafe { GetAce(dacl, 0, &mut ace_ptr) };
        assert_ne!(ok, 0, "GetAce failed");
        // SAFETY: `ace_ptr` was populated by the successful `GetAce` call
        // above and points to an `ACCESS_ALLOWED_ACE`-shaped structure for
        // the SDDL this crate generates ("D:P(A;;FA;;;OW)").
        let mask = unsafe { (*(ace_ptr as *const ACCESS_ALLOWED_ACE)).Mask };
        assert_eq!(
            mask, FILE_ALL_ACCESS,
            "owner ACE must grant full access, got mask {mask:#x}"
        );

        // SAFETY: `sd` was allocated by `GetNamedSecurityInfoW` and must
        // be freed with `LocalFree` per its documented contract.
        unsafe {
            LocalFree(sd);
        }
    }
}

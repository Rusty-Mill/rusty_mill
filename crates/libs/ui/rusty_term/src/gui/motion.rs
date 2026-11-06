//! Reduced motion (pack D27, D50 to D52): whether the chrome should hold
//! still. Under reduced motion the cursor draws solid instead of blinking
//! and leaves no trail; program output (Kitty image animations) still plays,
//! since it is content, not chrome.
//!
//! The `[window] reduce_motion` config key decides: `on` or `off` force it,
//! and absent ("system") follows the OS accessibility setting, read once per
//! process. No OS signal (a missing tool, an error, odd output) means motion
//! is allowed; `reduce_motion = "on"` covers those desktops.

use std::sync::OnceLock;

use crate::config::ReduceMotion;

/// Whether to reduce motion, given the configured override and a probe of
/// the OS setting (only called for "system").
pub(crate) fn resolve(pref: Option<ReduceMotion>, os: impl FnOnce() -> bool) -> bool {
    match pref {
        Some(ReduceMotion::On) => true,
        Some(ReduceMotion::Off) => false,
        None => os(),
    }
}

/// The OS reduced-motion setting, read on first use and cached for the
/// process (D52: startup only; a live re-read is a later follow-up).
pub(crate) fn os_reduces_motion() -> bool {
    static OS: OnceLock<bool> = OnceLock::new();
    *OS.get_or_init(probe)
}

#[cfg(windows)]
fn probe() -> bool {
    use windows_sys::Win32::Foundation::BOOL;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    };
    let mut animate: BOOL = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL through `pvparam`,
    // which points at a live, correctly typed local.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut animate as *mut BOOL).cast(),
            0,
        )
    };
    ok != 0 && animate == 0
}

#[cfg(target_os = "macos")]
fn probe() -> bool {
    command_output(
        "defaults",
        &["read", "com.apple.universalaccess", "reduceMotion"],
    )
    .and_then(|out| parse_defaults(&out))
    .unwrap_or(false)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn probe() -> bool {
    command_output(
        "gsettings",
        &["get", "org.gnome.desktop.interface", "enable-animations"],
    )
    .and_then(|out| parse_gsettings(&out))
    .unwrap_or(false)
}

#[cfg(not(any(windows, unix)))]
fn probe() -> bool {
    false
}

/// Run a settings reader and return its stdout, or `None` if it is missing,
/// fails, or takes longer than a startup can afford (it is killed then).
#[cfg(unix)]
fn command_output(program: &str, args: &[&str]) -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const LIMIT: Duration = Duration::from_millis(300);
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().ok()? {
            break status;
        }
        if start.elapsed() > LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if !status.success() {
        return None;
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    Some(out)
}

/// `gsettings get org.gnome.desktop.interface enable-animations` prints
/// `true` or `false`; animations off means reduce.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
fn parse_gsettings(out: &str) -> Option<bool> {
    match out.trim() {
        "false" => Some(true),
        "true" => Some(false),
        _ => None,
    }
}

/// `defaults read com.apple.universalaccess reduceMotion` prints `1` or `0`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_defaults(out: &str) -> Option<bool> {
    match out.trim() {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_override_wins_and_system_asks_the_os() {
        let never = || panic!("an override must not probe the OS");
        assert!(resolve(Some(ReduceMotion::On), never));
        assert!(!resolve(Some(ReduceMotion::Off), never));
        assert!(resolve(None, || true));
        assert!(!resolve(None, || false));
    }

    #[test]
    fn gsettings_animations_off_means_reduce() {
        assert_eq!(parse_gsettings("false\n"), Some(true));
        assert_eq!(parse_gsettings("true\n"), Some(false));
        assert_eq!(parse_gsettings(""), None);
        assert_eq!(parse_gsettings("No such schema"), None);
    }

    #[test]
    fn defaults_one_means_reduce() {
        assert_eq!(parse_defaults("1\n"), Some(true));
        assert_eq!(parse_defaults("0\n"), Some(false));
        assert_eq!(
            parse_defaults("The domain/default pair does not exist"),
            None
        );
    }
}

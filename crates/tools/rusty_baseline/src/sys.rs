//! Per-OS process memory: the peak resident set of an exited child, and
//! the current resident set of a running one. Targets without a backend
//! report `None` rather than guess.

use std::io;
use std::process::Child;

/// A running process's resident set, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memory {
    pub current: u64,
    pub peak: u64,
}

/// How a measured child ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    Exited(i32),
    /// Killed by a signal (Unix), or no exit code available.
    Killed,
}

/// Lowers what a child's peak RSS can read as, before spawning it.
///
/// Linux's `ru_maxrss` for a child counts the spawner's own peak at the
/// moment of `exec` (the pre-exec address space is the spawner's). Resetting
/// the spawner's peak to its current RSS (`clear_refs` 5) leaves that as the
/// floor, which [`floor`] reports. Best effort: no reset, a higher floor.
#[cfg(target_os = "linux")]
pub fn reset_peak() {
    let _ = std::fs::write("/proc/self/clear_refs", "5");
}

/// The lowest peak RSS a child can report, in bytes, measured on `true`.
#[cfg(target_os = "linux")]
pub fn floor() -> Option<u64> {
    reset_peak();
    let mut child = std::process::Command::new("true").spawn().ok()?;
    wait_with_peak(&mut child).ok()?.1
}

/// Waits for `child` and returns how it ended plus its peak RSS in bytes.
#[cfg(target_os = "linux")]
pub fn wait_with_peak(child: &mut Child) -> io::Result<(Ended, Option<u64>)> {
    use rusty_libc::wait::{waitpid_rusage, wexitstatus, wifexited};
    let pid = i32::try_from(child.id()).map_err(io::Error::other)?;
    // Reaps the child here, so `child.wait()` must not be called after.
    let (_, status, rusage) =
        waitpid_rusage(pid, 0).map_err(|errno| io::Error::other(format!("wait4: {errno:?}")))?;
    let ended = if wifexited(status) {
        Ended::Exited(wexitstatus(status))
    } else {
        Ended::Killed
    };
    // `ru_maxrss` is in KiB on Linux.
    Ok((
        ended,
        u64::try_from(rusage.maxrss).ok().map(|kib| kib * 1024),
    ))
}

/// Running `child`'s current and peak RSS, in bytes. Read from the process
/// itself, so unlike [`wait_with_peak`] there is no spawner floor.
#[cfg(target_os = "linux")]
pub fn memory(child: &Child) -> Option<Memory> {
    let status = std::fs::read_to_string(format!("/proc/{}/status", child.id())).ok()?;
    Some(Memory {
        current: kib_field(&status, "VmRSS:")?,
        peak: kib_field(&status, "VmHWM:")?,
    })
}

/// A `kB` field (e.g. `VmRSS:`) of a `/proc/<pid>/status` body, in bytes.
#[cfg(any(target_os = "linux", test))]
fn kib_field(status: &str, name: &str) -> Option<u64> {
    let line = status.lines().find(|line| line.starts_with(name))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(kib * 1024)
}

/// Windows counts a child's working set from its own start: no floor.
#[cfg(not(target_os = "linux"))]
pub fn reset_peak() {}

#[cfg(not(target_os = "linux"))]
pub fn floor() -> Option<u64> {
    None
}

#[cfg(windows)]
pub fn wait_with_peak(child: &mut Child) -> io::Result<(Ended, Option<u64>)> {
    let status = child.wait()?;
    let ended = status.code().map_or(Ended::Killed, Ended::Exited);
    // The handle stays open until `child` drops, so the exited process's
    // counters are still readable.
    Ok((ended, memory(child).map(|memory| memory.peak)))
}

#[cfg(windows)]
pub fn memory(child: &Child) -> Option<Memory> {
    use std::os::windows::io::AsRawHandle;
    // SAFETY: `child` owns this process handle and keeps it open for the
    // duration of the borrow.
    let memory = unsafe { rusty_win32::process::memory(child.as_raw_handle()) }.ok()?;
    Some(Memory {
        current: memory.working_set as u64,
        peak: memory.peak_working_set as u64,
    })
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn wait_with_peak(child: &mut Child) -> io::Result<(Ended, Option<u64>)> {
    let status = child.wait()?;
    Ok((status.code().map_or(Ended::Killed, Ended::Exited), None))
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn memory(_child: &Child) -> Option<Memory> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_kib_fields_in_bytes_and_tolerates_their_absence() {
        let status = "Name:\trush\nVmHWM:\t    9000 kB\nVmRSS:\t    8192 kB\n";
        assert_eq!(kib_field(status, "VmRSS:"), Some(8192 * 1024));
        assert_eq!(kib_field(status, "VmHWM:"), Some(9000 * 1024));
        assert_eq!(kib_field("Name:\tzombie\n", "VmRSS:"), None);
    }
}

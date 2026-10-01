//! Per-OS process memory: the peak resident set of an exited child, and
//! the current resident set of a running one. Targets without a backend
//! report `None` rather than guess.

use std::io;
use std::process::Child;

/// How a measured child ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    Exited(i32),
    /// Killed by a signal (Unix), or no exit code available.
    Killed,
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

/// The current RSS of running `child`, in bytes.
#[cfg(target_os = "linux")]
pub fn current_rss(child: &Child) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{}/status", child.id())).ok()?;
    vm_rss(&status)
}

/// `VmRSS` from a `/proc/<pid>/status` body, in bytes.
#[cfg(any(target_os = "linux", test))]
fn vm_rss(status: &str) -> Option<u64> {
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(kib * 1024)
}

#[cfg(windows)]
pub fn wait_with_peak(child: &mut Child) -> io::Result<(Ended, Option<u64>)> {
    let status = child.wait()?;
    let ended = status.code().map_or(Ended::Killed, Ended::Exited);
    // The handle stays open until `child` drops, so the exited process's
    // counters are still readable.
    Ok((
        ended,
        memory(child).map(|memory| memory.peak_working_set as u64),
    ))
}

#[cfg(windows)]
pub fn current_rss(child: &Child) -> Option<u64> {
    memory(child).map(|memory| memory.working_set as u64)
}

#[cfg(windows)]
fn memory(child: &Child) -> Option<rusty_win32::process::ProcessMemory> {
    use std::os::windows::io::AsRawHandle;
    // SAFETY: `child` owns this process handle and keeps it open for the
    // duration of the borrow.
    unsafe { rusty_win32::process::memory(child.as_raw_handle()) }.ok()
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn wait_with_peak(child: &mut Child) -> io::Result<(Ended, Option<u64>)> {
    let status = child.wait()?;
    Ok((status.code().map_or(Ended::Killed, Ended::Exited), None))
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn current_rss(_child: &Child) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_vm_rss_in_bytes_and_tolerates_its_absence() {
        let status = "Name:\trush\nVmHWM:\t    9000 kB\nVmRSS:\t    8192 kB\n";
        assert_eq!(vm_rss(status), Some(8192 * 1024));
        assert_eq!(vm_rss("Name:\tzombie\n"), None);
    }
}

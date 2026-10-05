//! Running a built product: startup time and resident memory.

use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::products::{Mode, Product};
use crate::sys::{self, Ended};

/// What running a product measured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Run {
    /// An `exit`-mode product: median wall time to exit, max peak RSS.
    Exit {
        startup: Duration,
        peak_rss: Option<u64>,
        ended: Ended,
    },
    /// An `idle`-mode product: RSS once settled, and its peak so far.
    Idle {
        idle_rss: Option<u64>,
        peak_rss: Option<u64>,
    },
}

/// Runs the product at `binary` per its mode. `home` stands in for the
/// user's home and data directories, so no run reads or writes real ones.
/// Every exit-mode sample, including the warm-up, must match the product's
/// expected code. A mismatch invalidates the run rather than contributing a
/// failed command's timing to the startup median.
pub fn run(
    product: &Product,
    binary: &Path,
    home: &Path,
    runs: usize,
    settle: Duration,
) -> Result<Run, String> {
    match product.mode {
        Mode::Exit => run_to_exit(product, binary, home, runs),
        Mode::Idle => run_idle(product, binary, home, settle),
    }
}

fn run_to_exit(product: &Product, binary: &Path, home: &Path, runs: usize) -> Result<Run, String> {
    // One untimed warm-up, so every timed run starts with a warm page cache.
    once(product, binary, home).map_err(|error| format!("warm-up {error}"))?;
    let mut times = Vec::with_capacity(runs);
    let mut peak_rss = None;
    let mut ended = Ended::Killed;
    for index in 0..runs {
        let started = Instant::now();
        let (end, peak) = once(product, binary, home)
            .map_err(|error| format!("timed run {} {error}", index + 1))?;
        times.push(started.elapsed());
        peak_rss = peak_rss.max(peak);
        ended = end;
    }
    Ok(Run::Exit {
        startup: median(&mut times).ok_or("no timed runs (--runs 0)")?,
        peak_rss,
        ended,
    })
}

fn once(product: &Product, binary: &Path, home: &Path) -> Result<(Ended, Option<u64>), String> {
    let mut child = spawn(product, binary, home)?;
    let (ended, peak) = sys::wait_with_peak(&mut child)
        .map_err(|error| format!("waiting for {}: {error}", product.bin))?;
    validate_exit(
        ended,
        product.expected_exit.code_for_os(std::env::consts::OS),
    )?;
    Ok((ended, peak))
}

fn validate_exit(ended: Ended, expected: i32) -> Result<(), String> {
    if ended != Ended::Exited(expected) {
        return Err(format!("ended {ended:?}; expected exit {expected}"));
    }
    Ok(())
}

fn run_idle(
    product: &Product,
    binary: &Path,
    home: &Path,
    settle: Duration,
) -> Result<Run, String> {
    let mut child = spawn(product, binary, home)?;
    std::thread::sleep(settle);
    let status = child.try_wait();
    finish_idle(&product.bin, &mut child, status)
}

fn finish_idle(
    bin: &str,
    child: &mut Child,
    status: std::io::Result<Option<ExitStatus>>,
) -> Result<Run, String> {
    match status {
        Ok(Some(status)) => {
            return Err(format!(
                "{bin} exited ({status}) before settling; check its args"
            ));
        }
        Ok(None) => {}
        Err(error) => {
            let mut error = format!("checking {bin} after settling: {error}");
            // A failed poll says nothing about whether the child is alive.
            // Still attempt teardown, retaining both errors if it also fails.
            if let Err(cleanup) = stop_idle(bin, child) {
                error.push_str(&format!("; cleanup: {cleanup}"));
            }
            return Err(error);
        }
    }
    // Read before the kill: the process's own counters, no spawner floor.
    let memory = sys::memory(child);
    stop_idle(bin, child)?;
    Ok(Run::Idle {
        idle_rss: memory.map(|memory| memory.current),
        peak_rss: memory.map(|memory| memory.peak),
    })
}

fn stop_idle(bin: &str, child: &mut Child) -> Result<(), String> {
    child
        .kill()
        .map_err(|error| format!("killing {bin}: {error}"))?;
    child
        .wait()
        .map_err(|error| format!("waiting for {bin}: {error}"))?;
    Ok(())
}

fn spawn(product: &Product, binary: &Path, home: &Path) -> Result<Child, String> {
    let mut command = Command::new(binary);
    command
        .args(&product.args)
        .current_dir(home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for variable in HOME_VARIABLES {
        command.env(variable, home);
    }
    command.envs(product.env.iter().map(|(key, value)| (key, value)));
    sys::reset_peak();
    command
        .spawn()
        .map_err(|error| format!("starting {}: {error}", binary.display()))
}

/// Every variable a product might derive its home, config, data or cache
/// directory from, on Unix and Windows.
const HOME_VARIABLES: [&str; 8] = [
    "HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
];

/// The median of `times` (the upper one of an even count), or `None` if empty.
fn median(times: &mut [Duration]) -> Option<Duration> {
    times.sort_unstable();
    times.get(times.len() / 2).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_poll_error_is_reported_and_child_reaped() {
        // Re-execute this test as an owned idle process, without a shell or
        // platform-specific executable. Only the child receives this flag.
        const CHILD: &str = "RUSTY_BASELINE_IDLE_POLL_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            loop {
                std::thread::park();
            }
        }
        for fail_poll in [false, true] {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "measure::tests::idle_poll_error_is_reported_and_child_reaped",
                ])
                .env(CHILD, "1")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            assert!(child.try_wait().unwrap().is_none());
            let status = if fail_poll {
                Err(std::io::Error::other("injected poll failure"))
            } else {
                Ok(None)
            };
            let result = finish_idle("fixture", &mut child, status);
            let stopped = child.try_wait().unwrap().is_some();
            if !stopped {
                // Keep even a regressed test from leaving its child alive.
                child.kill().unwrap();
                child.wait().unwrap();
            }
            assert!(stopped, "idle child was not killed and reaped");
            if fail_poll {
                assert_eq!(
                    result.unwrap_err(),
                    "checking fixture after settling: injected poll failure"
                );
            } else {
                assert!(matches!(result, Ok(Run::Idle { .. })));
            }
        }
    }

    #[test]
    fn only_the_exact_declared_exit_code_is_valid() {
        assert!(validate_exit(Ended::Exited(0), 0).is_ok());
        assert!(validate_exit(Ended::Exited(2), 2).is_ok());
        assert!(validate_exit(Ended::Exited(0), 2).is_err());
        assert!(validate_exit(Ended::Exited(7), 0).is_err());
        assert!(validate_exit(Ended::Killed, 0).is_err());
        assert!(validate_exit(Ended::Killed, 2).is_err());
    }

    #[test]
    fn median_is_order_independent_and_none_when_empty() {
        let ms = Duration::from_millis;
        assert_eq!(median(&mut [ms(9), ms(1), ms(5)]), Some(ms(5)));
        assert_eq!(median(&mut [ms(4), ms(2)]), Some(ms(4)));
        assert_eq!(median(&mut []), None);
    }
}

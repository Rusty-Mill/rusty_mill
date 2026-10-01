//! Running a built product: startup time and resident memory.

use std::path::Path;
use std::process::{Child, Command, Stdio};
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
    /// An `idle`-mode product: RSS once settled, and its peak until killed.
    Idle {
        idle_rss: Option<u64>,
        peak_rss: Option<u64>,
    },
}

/// Runs the product at `binary` per its mode. `home` stands in for the
/// user's home and data directories, so no run reads or writes real ones.
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
    once(product, binary, home)?;
    let mut times = Vec::with_capacity(runs);
    let mut peak_rss = None;
    let mut ended = Ended::Killed;
    for _ in 0..runs {
        let started = Instant::now();
        let (end, peak) = once(product, binary, home)?;
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
    sys::wait_with_peak(&mut child).map_err(|error| format!("waiting for {}: {error}", product.bin))
}

fn run_idle(
    product: &Product,
    binary: &Path,
    home: &Path,
    settle: Duration,
) -> Result<Run, String> {
    let mut child = spawn(product, binary, home)?;
    std::thread::sleep(settle);
    if let Ok(Some(status)) = child.try_wait() {
        return Err(format!(
            "{} exited ({status}) before settling; check its args",
            product.bin
        ));
    }
    let idle_rss = sys::current_rss(&child);
    child
        .kill()
        .map_err(|error| format!("killing {}: {error}", product.bin))?;
    let (_, peak_rss) = sys::wait_with_peak(&mut child)
        .map_err(|error| format!("waiting for {}: {error}", product.bin))?;
    Ok(Run::Idle { idle_rss, peak_rss })
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
    fn median_is_order_independent_and_none_when_empty() {
        let ms = Duration::from_millis;
        assert_eq!(median(&mut [ms(9), ms(1), ms(5)]), Some(ms(5)));
        assert_eq!(median(&mut [ms(4), ms(2)]), Some(ms(4)));
        assert_eq!(median(&mut []), None);
    }
}

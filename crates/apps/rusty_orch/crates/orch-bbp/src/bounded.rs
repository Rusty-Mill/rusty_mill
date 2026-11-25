//! Child processes with a time limit (#1318 S5 review): every helper orch
//! runs before or after a builder turn (git, `bbp`, `rusty-mux-ctl`) must
//! answer within a bound.

use std::io::Read;
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// `cmd`'s output, or an error once `wait` has passed, so a stuck helper
/// cannot hold its caller forever. `cmd` leads its own process group, and
/// on a timeout the whole group is killed, so a grandchild holding a pipe
/// goes too and the reader threads end.
///
/// # Errors
///
/// When `cmd` cannot be spawned or waited on, or `wait` passes.
pub fn output_within(mut cmd: Command, wait: Duration) -> Result<std::process::Output, String> {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let until = Instant::now() + wait;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if Instant::now() >= until {
            // Not reaped yet, so the pid is still this child's.
            kill_group(&child);
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("no answer within {wait:?}"));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // A grandchild can hold a pipe open past the child's exit. While it
    // lives the group id can't be reused, so the group kill still hits it.
    let left = || until.saturating_duration_since(Instant::now());
    let (Ok(stdout), Ok(stderr)) = (out.recv_timeout(left()), err.recv_timeout(left())) else {
        kill_group(&child);
        return Err(format!("output still open after {wait:?}"));
    };
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// SIGKILL `child`'s process group, which it leads; best effort.
fn kill_group(child: &std::process::Child) {
    #[cfg(target_os = "linux")]
    if let Ok(pid) = i32::try_from(child.id()) {
        let _ = platform_linux::sys::spawn::kill_group(pid, platform::process::Signal::Kill);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = child;
}

/// Read `pipe` to its end on a thread; the bytes arrive on the receiver.
fn drain(pipe: Option<impl Read + Send + 'static>) -> Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut bytes);
        }
        let _ = tx.send(bytes);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quick_command_returns_its_output() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo out; echo err >&2"]);
        let out = output_within(cmd, Duration::from_secs(10)).expect("runs");
        assert!(out.status.success());
        assert_eq!(out.stdout, b"out\n");
        assert_eq!(out.stderr, b"err\n");
    }

    #[test]
    fn a_stuck_command_is_killed_at_the_bound() {
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        let start = Instant::now();
        let err = output_within(cmd, Duration::from_millis(200)).expect_err("times out");
        assert!(err.contains("no answer"), "{err}");
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_grandchild_holding_the_pipe_does_not_hang_it() {
        let mut cmd = Command::new("sh");
        let pid_file = std::env::temp_dir().join(format!("orch-bounded-{}", std::process::id()));
        cmd.arg("-c").arg(format!(
            "sleep 30 & echo $! > {}; echo started",
            pid_file.display()
        ));
        let start = Instant::now();
        let err = output_within(cmd, Duration::from_millis(500)).expect_err("pipe held");
        assert!(err.contains("still open"), "{err}");
        assert!(start.elapsed() < Duration::from_secs(5));
        // The group kill took the grandchild too.
        let pid = std::fs::read_to_string(&pid_file).expect("pid file");
        let _ = std::fs::remove_file(&pid_file);
        let gone = (0..50).any(|_| {
            std::thread::sleep(Duration::from_millis(20));
            // Gone, or a zombie waiting for its new parent to reap it.
            std::fs::read_to_string(format!("/proc/{}/stat", pid.trim()))
                .map_or(true, |stat| stat.contains(") Z "))
        });
        assert!(gone, "sleep {} still runs", pid.trim());
    }

    #[test]
    fn a_missing_program_is_an_error() {
        let cmd = Command::new("/nonexistent/orch-bounded-test");
        assert!(output_within(cmd, Duration::from_secs(1)).is_err());
    }
}

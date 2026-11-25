//! Child processes with a time limit (#1318 S5 review): every helper orch
//! runs before or after a builder turn (git, `bbp`, `rusty-mux-ctl`) must
//! answer within a bound.

use std::io::Read;
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// `cmd`'s output, or an error once `wait` has passed (it is killed then),
/// so a stuck helper cannot hold its caller forever.
///
/// # Errors
///
/// When `cmd` cannot be spawned or waited on, or `wait` passes.
pub fn output_within(mut cmd: Command, wait: Duration) -> Result<std::process::Output, String> {
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
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("no answer within {wait:?}"));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // A grandchild can hold a pipe open past the child's exit.
    let left = || until.saturating_duration_since(Instant::now());
    let stdout = out
        .recv_timeout(left())
        .map_err(|_| format!("output still open after {wait:?}"))?;
    let stderr = err
        .recv_timeout(left())
        .map_err(|_| format!("output still open after {wait:?}"))?;
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
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
    fn a_grandchild_holding_the_pipe_does_not_hang_it() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 30 & echo started"]);
        let start = Instant::now();
        let err = output_within(cmd, Duration::from_millis(500)).expect_err("pipe held");
        assert!(err.contains("still open"), "{err}");
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_missing_program_is_an_error() {
        let cmd = Command::new("/nonexistent/orch-bounded-test");
        assert!(output_within(cmd, Duration::from_secs(1)).is_err());
    }
}

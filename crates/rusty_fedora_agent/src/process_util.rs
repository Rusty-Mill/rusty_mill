//! A small helper for running a command through a `rustils` [`Spawner`]
//! and capturing its output. `rustils`' lower-level `Spawner`/`Child`/
//! `Stdio::Pipe` API has no `std::process::Command::output()`-style
//! convenience (see `platform::process`'s own doc comments -- capturing
//! output is a pipe/read/wait sequence a caller assembles itself), so
//! every adapter in this crate goes through this once rather than
//! repeating that sequence per call site.

use platform::error::{ErrorKind, OsCode, PlatformError};
use platform::process::{Command, ExitStatus, Spawner, Stdio};

use crate::error::AgentError;

/// A finished command's outcome.
pub struct CapturedOutput {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

/// Runs `program` with `args` via `spawner`, capturing stdout and stderr
/// as UTF-8 (lossily -- every command this agent runs produces text).
///
/// Reads stderr to completion before stdout: for this agent's fixed
/// command shapes (`systemctl`, `journalctl`, `dnf`), stderr is normally a
/// handful of warning/error lines at most, while stdout can be large (a
/// journal dump, a dnf transaction list) -- draining the smaller stream
/// first keeps the larger one from ever backing up behind it. A
/// pathological command that inverted that assumption (large stderr *and*
/// large stdout, neither drained) could still deadlock on the pipe buffer;
/// none of this agent's call sites do that.
pub fn run_captured(
    spawner: &dyn Spawner,
    op: &'static str,
    program: &str,
    args: &[String],
) -> Result<CapturedOutput, AgentError> {
    let mut cmd = Command::new(program, "/").args(args.iter().cloned());
    cmd.stdout = Stdio::Pipe;
    cmd.stderr = Stdio::Pipe;

    let mut child = spawner
        .spawn(&cmd)
        .map_err(|source| platform_err(op, source))?;

    let mut stderr_pipe = child
        .take_stderr()
        .ok_or_else(|| platform_err(op, missing_pipe(op)))?;
    let stderr_bytes = read_all(stderr_pipe.as_mut()).map_err(|source| platform_err(op, source))?;
    drop(stderr_pipe);

    let mut stdout_pipe = child
        .take_stdout()
        .ok_or_else(|| platform_err(op, missing_pipe(op)))?;
    let stdout_bytes = read_all(stdout_pipe.as_mut()).map_err(|source| platform_err(op, source))?;
    drop(stdout_pipe);

    let status = child.wait().map_err(|source| platform_err(op, source))?;

    Ok(CapturedOutput {
        status,
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
    })
}

/// Runs `program`/`args` and turns a non-zero exit into
/// [`AgentError::CommandFailed`], returning just the captured stdout on
/// success -- the common case for this agent's read-only shellouts
/// (`systemctl list-units`, `journalctl`, `dnf check-update`).
pub fn run_checked(
    spawner: &dyn Spawner,
    op: &'static str,
    program: &str,
    args: &[String],
) -> Result<String, AgentError> {
    let output = run_captured(spawner, op, program, args)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(AgentError::CommandFailed {
            op,
            exit_code: match output.status {
                ExitStatus::Code(code) => Some(code),
                _ => None,
            },
            stderr: output.stderr,
        })
    }
}

fn read_all(file: &mut dyn platform::fs::File) -> platform::error::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    Ok(out)
}

/// Synthesizes a `PlatformError` for the "the backend didn't hand back a
/// piped stdio slot we explicitly requested" case -- an internal
/// invariant violation on every backend this crate targets, not a real OS
/// failure, but still worth a typed error over a panic.
fn missing_pipe(op: &'static str) -> PlatformError {
    PlatformError::new(ErrorKind::Other, OsCode::None, op)
}

fn platform_err(op: &'static str, source: PlatformError) -> AgentError {
    AgentError::Platform { op, source }
}

#[cfg(test)]
mod tests {
    use super::*;
    use platform::process::ExitStatus;
    use platform_mock::process::MockSpawner;

    #[test]
    fn a_successful_command_returns_captured_stdout() {
        let spawner = MockSpawner::new().script_with_output(
            "systemctl",
            ExitStatus::Code(0),
            b"active\n".to_vec(),
        );

        let output = run_checked(
            &spawner,
            "systemctl",
            "systemctl",
            &["is-active".to_string()],
        )
        .expect("mocked command succeeds");
        assert_eq!(output, "active\n");
    }

    #[test]
    fn a_failing_command_becomes_a_command_failed_error() {
        let spawner = MockSpawner::new().script("systemctl", ExitStatus::Code(1));

        let err = run_checked(&spawner, "systemctl", "systemctl", &["start".to_string()])
            .expect_err("mocked command fails");
        assert!(matches!(
            err,
            AgentError::CommandFailed {
                exit_code: Some(1),
                ..
            }
        ));
    }

    #[test]
    fn an_unscripted_program_surfaces_as_a_platform_error() {
        let spawner = MockSpawner::new();

        let err = run_checked(&spawner, "systemctl", "systemctl", &["start".to_string()])
            .expect_err("no script registered for this program");
        assert!(matches!(err, AgentError::Platform { .. }));
    }
}

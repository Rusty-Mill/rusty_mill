//! Claude is two processes per run: the login probe, then `claude -p`. The
//! fake here answers the probe from one canned result and everything else
//! from another, recording both through `orch_cli`'s fake.
#![allow(dead_code)]

pub use orch_cli::fake::*;

use std::path::Path;
use std::time::Duration;

use orch_claude::ClaudeAgent;
use orch_cli::{CommandRunner, ExecError, Exit};

/// A result envelope as `claude -p --output-format json --json-schema`
/// prints it, with `structured` as the validated object.
pub fn envelope(structured: &str) -> String {
    format!(
        r#"{{"type":"result","subtype":"success","is_error":false,"num_turns":3,"session_id":"s","result":"(text)","structured_output":{structured}}}"#
    )
}

/// An envelope that reports a failure instead of a reply.
pub fn error_envelope(subtype: &str, result: &str) -> String {
    format!(
        r#"{{"type":"result","subtype":"{subtype}","is_error":true,"num_turns":1,"session_id":"s","result":"{result}"}}"#
    )
}

/// Scripted process pair: `auth` answers the login probe, `main` the run.
pub struct Scripted {
    pub inner: FakeCommand,
    auth: Result<Exit, ExecError>,
    main: Result<Exit, ExecError>,
}

impl Scripted {
    /// Logged in; the run exits 0 with `stdout`.
    pub fn ok(stdout: &str) -> Self {
        Self::main(Ok(exit(0, stdout, "")))
    }

    /// Logged in; the run ends as `result`.
    pub fn main(result: Result<Exit, ExecError>) -> Self {
        Self {
            inner: FakeCommand::ok(""),
            auth: Ok(exit(0, r#"{"loggedIn":true}"#, "")),
            main: result,
        }
    }

    /// The probe ends as `result`; the run would succeed if reached.
    pub fn auth(result: Result<Exit, ExecError>) -> Self {
        Self {
            inner: FakeCommand::ok(""),
            auth: result,
            main: Ok(exit(0, &envelope(REPLY_ONE), "")),
        }
    }
}

impl CommandRunner for Scripted {
    fn run_in(
        &self,
        cwd: Option<&Path>,
        argv: &[String],
        stdin: &[u8],
        timeout: Duration,
        remove_env: &[&str],
    ) -> Result<Exit, ExecError> {
        self.inner.run_in(cwd, argv, stdin, timeout, remove_env)?;
        if argv == ClaudeAgent::<Scripted>::auth_argv().as_slice() {
            self.auth.clone()
        } else {
            self.main.clone()
        }
    }
}

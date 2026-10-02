//! Codex reads its reply from the `--output-last-message` file, so the fake
//! runner here writes that file before delegating to `orch_cli`'s fake.
#![allow(dead_code)]

pub use orch_cli::fake::*;

use std::time::Duration;

use orch_cli::{CommandRunner, ExecError, Exit};

/// Wraps [`FakeCommand`]: writes `reply` to the path after
/// `--output-last-message` in the argv, then returns the canned result.
pub struct ReplyFile {
    pub inner: FakeCommand,
    pub reply: Option<String>,
}

impl ReplyFile {
    pub fn ok(reply: &str) -> Self {
        Self {
            inner: FakeCommand::ok(""),
            reply: Some(reply.to_owned()),
        }
    }

    pub fn failing(result: Result<Exit, ExecError>) -> Self {
        Self {
            inner: FakeCommand::new(result),
            reply: None,
        }
    }

    /// Exit 0 but no last-message file written.
    pub fn silent() -> Self {
        Self {
            inner: FakeCommand::ok(""),
            reply: None,
        }
    }
}

impl CommandRunner for ReplyFile {
    fn run_scrubbed(
        &self,
        argv: &[String],
        stdin: &[u8],
        timeout: Duration,
        remove_env: &[&str],
    ) -> Result<Exit, ExecError> {
        if let Some(reply) = &self.reply {
            let path = argv
                .iter()
                .position(|a| a == "--output-last-message")
                .and_then(|i| argv.get(i + 1))
                .expect("argv names the last-message file");
            std::fs::write(path, reply).expect("write reply file");
        }
        self.inner.run_scrubbed(argv, stdin, timeout, remove_env)
    }
}

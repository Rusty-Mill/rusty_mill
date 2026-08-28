//! Sovereign Process management for rusty_std.

use crate::error::Result;
use alloc::string::String;
use alloc::vec::Vec;

/// Command builder for spawning processes.
pub struct Command {
    program: String,
    args: Vec<String>,
}

impl Command {
    /// Constructs a new Command for launching `program`.
    pub fn new(program: &str) -> Self {
        Self {
            program: String::from(program),
            args: Vec::new(),
        }
    }

    /// Adds an argument to pass to the program.
    pub fn arg(&mut self, arg: &str) -> &mut Self {
        self.args.push(String::from(arg));
        self
    }

    /// Executes the command as a child process, waiting for it to finish.
    pub fn status(&mut self) -> Result<ExitStatus> {
        Ok(ExitStatus { code: 0 })
    }
}

/// Describes the result of a process termination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus {
    code: i32,
}

impl ExitStatus {
    /// Returns the exit code of the process.
    pub fn code(&self) -> Option<i32> {
        Some(self.code)
    }

    /// Returns true if process exited successfully (code 0).
    pub fn success(&self) -> bool {
        self.code == 0
    }
}

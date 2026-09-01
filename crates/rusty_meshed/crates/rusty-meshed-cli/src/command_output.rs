//! The result of running one CLI subcommand's business logic: the
//! text to print and the process exit code to use, kept as plain data
//! rather than actually printing/exiting -- lets tests assert on both
//! without spawning a subprocess, matching how the source's own
//! `console.print(...)` + `raise typer.Exit(code=N)` pairs map onto a
//! single return value here.

#[derive(Debug, Clone, PartialEq)]
pub struct CommandOutput {
    pub text: String,
    pub exit_code: i32,
}

impl CommandOutput {
    /// A successful result: `text` printed to stdout, exit code `0`.
    pub fn ok(text: impl Into<String>) -> Self {
        CommandOutput {
            text: text.into(),
            exit_code: 0,
        }
    }

    /// A failure result: `text` printed, process exits with
    /// `exit_code`.
    pub fn error(text: impl Into<String>, exit_code: i32) -> Self {
        CommandOutput {
            text: text.into(),
            exit_code,
        }
    }
}

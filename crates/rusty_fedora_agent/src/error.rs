//! Everything that can go wrong inside the agent.
//!
//! Reuses `rustils`' [`PlatformError`] for process/spawn failures rather
//! than reinventing it -- this type only adds the failure modes that are
//! specific to this agent's own scoping and command-shape rules.

use platform::error::PlatformError;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    /// A `rustils` platform operation (spawn, resolve, ...) failed.
    #[error("platform error in {op}")]
    Platform {
        op: &'static str,
        #[source]
        source: PlatformError,
    },

    /// The requested unit is not in the configured unit allowlist. Checked
    /// before a `systemctl` command is ever built.
    #[error("unit '{0}' is not in the allowlist")]
    UnitNotAllowed(String),

    /// The requested package is not in the configured package allowlist.
    /// Checked before a `dnf` command is ever built.
    #[error("package '{0}' is not in the allowlist")]
    PackageNotAllowed(String),

    /// The requested path is not under any configured config-path prefix,
    /// or attempts to escape one via `..`. Checked before the filesystem
    /// is touched.
    #[error("path '{0}' is not in the allowlist")]
    PathNotAllowed(String),

    /// A subprocess (`systemctl`, `dnf`, `journalctl`) exited non-zero.
    #[error("{op} failed (exit {exit_code:?}): {stderr}")]
    CommandFailed {
        op: &'static str,
        exit_code: Option<i32>,
        stderr: String,
    },

    /// `dnf`'s output didn't match the shape this agent expects to parse.
    #[error("dnf output could not be parsed: {0}")]
    DnfParse(String),

    /// `fedora_task_status` was asked about a task id this agent has never
    /// issued (or one from a previous, since-restarted process -- the task
    /// registry is in-memory only).
    #[error("no task with id '{0}'")]
    UnknownTask(String),

    /// The request itself was malformed (bad JSON, an out-of-range field,
    /// an unparseable allowlist config), independent of allowlisting.
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// A filesystem operation outside the `platform` crate's own surface
    /// (reading the allowlist config at startup, `.bak` copies) failed.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl AgentError {
    /// The HTTP status this error should be reported as.
    pub fn http_status(&self) -> u16 {
        match self {
            AgentError::UnitNotAllowed(_)
            | AgentError::PackageNotAllowed(_)
            | AgentError::PathNotAllowed(_)
            | AgentError::InvalidRequest(_) => 400,
            AgentError::UnknownTask(_) => 404,
            AgentError::Platform { .. } | AgentError::CommandFailed { .. } | AgentError::DnfParse(_) | AgentError::Io(_) => {
                500
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_rejections_are_client_errors() {
        assert_eq!(AgentError::UnitNotAllowed("x".into()).http_status(), 400);
        assert_eq!(AgentError::PackageNotAllowed("x".into()).http_status(), 400);
        assert_eq!(AgentError::PathNotAllowed("x".into()).http_status(), 400);
    }

    #[test]
    fn unknown_task_is_not_found() {
        assert_eq!(AgentError::UnknownTask("t1".into()).http_status(), 404);
    }

    #[test]
    fn command_failure_is_a_server_error() {
        let err = AgentError::CommandFailed {
            op: "systemctl",
            exit_code: Some(1),
            stderr: "unit not found".into(),
        };
        assert_eq!(err.http_status(), 500);
    }
}

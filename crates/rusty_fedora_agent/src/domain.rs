//! Value types the [`crate::ports::SystemController`]/
//! [`crate::ports::PackageController`] ports return, and that
//! [`crate::http`] serializes straight to JSON. No I/O here -- domain
//! logic stays free of process/HTTP details (ports-and-adapters).

use std::fmt;

use serde::{Deserialize, Serialize};

/// A systemctl action, as requested by `rusty_homelab_mcp`'s
/// `fedora_service_control` tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
    Enable,
    Disable,
}

impl ServiceAction {
    /// The `systemctl` subcommand for this action.
    pub fn as_systemctl_verb(self) -> &'static str {
        match self {
            ServiceAction::Start => "start",
            ServiceAction::Stop => "stop",
            ServiceAction::Restart => "restart",
            ServiceAction::Enable => "enable",
            ServiceAction::Disable => "disable",
        }
    }
}

/// Which systemd unit type `fedora_list_services` should list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitType {
    Service,
    Timer,
    Socket,
}

impl UnitType {
    /// The unit-name suffix `systemctl list-units --type=` takes.
    pub fn as_systemctl_type(self) -> &'static str {
        match self {
            UnitType::Service => "service",
            UnitType::Timer => "timer",
            UnitType::Socket => "socket",
        }
    }
}

/// One unit's summary line, as `systemctl list-units` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSummary {
    pub name: String,
    pub load_state: String,
    pub active_state: String,
    pub sub_state: String,
}

/// A `journalctl -p` priority filter, by name (`journalctl` accepts both
/// the numeric syslog level and this name; the name is what a caller
/// reads/writes without an RFC 5424 table open).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Emerg,
    Alert,
    Crit,
    Err,
    Warning,
    Notice,
    Info,
    Debug,
}

impl Priority {
    /// The value `journalctl -p` takes for this priority.
    pub fn as_journalctl_value(self) -> &'static str {
        match self {
            Priority::Emerg => "emerg",
            Priority::Alert => "alert",
            Priority::Crit => "crit",
            Priority::Err => "err",
            Priority::Warning => "warning",
            Priority::Notice => "notice",
            Priority::Info => "info",
            Priority::Debug => "debug",
        }
    }
}

/// Query parameters for `fedora_read_journal`. `unit: None` reads the full
/// system journal.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct JournalQuery {
    pub unit: Option<String>,
    pub lines: Option<u32>,
    pub since: Option<String>,
    pub priority: Option<Priority>,
}

/// One line of journal output (`journalctl -o short-iso` text, unparsed --
/// no consumer needs structured fields beyond the line itself yet).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalLine {
    pub line: String,
}

/// `fedora_system_status`'s payload: uptime, load, memory, kernel/release.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SystemStatus {
    pub hostname: String,
    pub kernel: String,
    pub os_pretty_name: String,
    pub uptime_seconds: u64,
    pub load_average_1m: f64,
    pub load_average_5m: f64,
    pub load_average_15m: f64,
    pub mem_total_kb: u64,
    pub mem_available_kb: u64,
}

/// One package with an update available, as `dnf check-update` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageUpdate {
    pub name: String,
    pub current_version: String,
    pub new_version: String,
    pub repo: String,
}

/// Identifies a long-running dnf task, handed back by
/// `fedora_dnf_install`/`fedora_dnf_remove` and polled via
/// `fedora_task_status`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub String);

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A task's run state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Running,
    Succeeded,
    Failed,
}

/// A dnf task's current status, as returned by `fedora_task_status`.
/// `stdout`/`stderr`/`exit_code` are `None` while [`TaskStatus::state`] is
/// still [`TaskState::Running`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskStatus {
    pub id: TaskId,
    pub state: TaskState,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub exit_code: Option<i32>,
}

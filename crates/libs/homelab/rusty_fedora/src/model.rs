//! Small enums describing service actions/types and journal priority --
//! everything else this crate hands back is `rusty_fedora_agent`'s own
//! JSON, unopinionated, mirroring how `rusty_opnsense`/`rusty_proxmox`
//! treat their own upstream response bodies.

use std::fmt;

/// A systemctl action, sent as `POST /services/{name}/control`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
    Enable,
    Disable,
}

impl ServiceAction {
    /// The value `rusty_fedora_agent`'s `/services/{name}/control` body
    /// takes for this action.
    pub fn as_str(self) -> &'static str {
        match self {
            ServiceAction::Start => "start",
            ServiceAction::Stop => "stop",
            ServiceAction::Restart => "restart",
            ServiceAction::Enable => "enable",
            ServiceAction::Disable => "disable",
        }
    }
}

impl fmt::Display for ServiceAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which systemd unit type to list, sent as `GET /services?unit_type=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitType {
    Service,
    Timer,
    Socket,
}

impl UnitType {
    /// The `unit_type` query value for this type.
    pub fn as_str(self) -> &'static str {
        match self {
            UnitType::Service => "service",
            UnitType::Timer => "timer",
            UnitType::Socket => "socket",
        }
    }
}

impl fmt::Display for UnitType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A `journalctl -p` priority filter, sent as `GET /journal?priority=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// The `priority` query value for this level.
    pub fn as_str(self) -> &'static str {
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

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

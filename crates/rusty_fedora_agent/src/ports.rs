//! Domain ports (ports-and-adapters): the trait boundary
//! [`crate::http`] calls through. Exists so tool handlers -- both this
//! crate's own HTTP layer and `rusty_homelab_mcp`'s tool tests -- can run
//! against a mock implementation without a real Fedora box. See
//! `crate::systemd::SystemdAdapter`/`crate::dnf::DnfController` for the
//! real, `rustils`-backed implementations.

use crate::domain::{
    JournalLine, JournalQuery, PackageUpdate, ServiceAction, ServiceSummary, SystemStatus,
    TaskId, TaskStatus, UnitType,
};
use crate::error::AgentError;

/// systemd read/control operations.
pub trait SystemController {
    fn list_services(
        &self,
        unit_type: Option<UnitType>,
    ) -> Result<Vec<ServiceSummary>, AgentError>;
    fn control_service(&self, name: &str, action: ServiceAction) -> Result<(), AgentError>;
    fn read_journal(&self, query: JournalQuery) -> Result<Vec<JournalLine>, AgentError>;
    fn system_status(&self) -> Result<SystemStatus, AgentError>;
}

/// dnf read/mutate operations. Installs/removes are asynchronous --
/// `install`/`remove` return a [`TaskId`] immediately; poll
/// [`PackageController::task_status`] for the outcome.
pub trait PackageController {
    fn list_updates(&self) -> Result<Vec<PackageUpdate>, AgentError>;
    fn install(&self, packages: &[String]) -> Result<TaskId, AgentError>;
    fn remove(&self, packages: &[String]) -> Result<TaskId, AgentError>;
    fn task_status(&self, id: &TaskId) -> Result<TaskStatus, AgentError>;
}

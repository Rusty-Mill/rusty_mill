//! OPNsense tools: system status, service control, interfaces, firewall
//! aliases, and gateways.

use rmcp::{Json, handler::server::wrapper::Parameters, model::ErrorData, tool, tool_router};
use rusty_mcp::ToolError;
use rusty_opnsense::ServiceAction;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::server::HomelabServer;

/// An action to take on a named service.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ServiceActionArg {
    /// Start the service.
    Start,
    /// Stop the service.
    Stop,
    /// Restart the service.
    Restart,
}

impl From<ServiceActionArg> for ServiceAction {
    fn from(value: ServiceActionArg) -> Self {
        match value {
            ServiceActionArg::Start => ServiceAction::Start,
            ServiceActionArg::Stop => ServiceAction::Stop,
            ServiceActionArg::Restart => ServiceAction::Restart,
        }
    }
}

/// Arguments for a service control call.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ServiceControlArgs {
    /// The service's short id, as returned by `opnsense_list_services`
    /// (e.g. `unbound`, `dhcpd`, `sshd`).
    pub name: String,
    /// The action to perform.
    pub action: ServiceActionArg,
}

#[tool_router(router = opnsense_tools, vis = "pub(crate)")]
impl HomelabServer {
    /// Firmware version, running kernel, pending updates, service health.
    #[tool(
        description = "Get the OPNsense firewall's overall system status: firmware version, running kernel, pending updates, and per-service health."
    )]
    pub async fn opnsense_system_status(&self) -> Result<Json<Value>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .system_status()
                .await
                .map_err(opnsense_error)?,
        ))
    }

    /// Every known service and its running state.
    #[tool(
        description = "List every service OPNsense's service supervisor knows about, with its running state and short id (used by opnsense_service_control)."
    )]
    pub async fn opnsense_list_services(&self) -> Result<Json<Value>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .list_services()
                .await
                .map_err(opnsense_error)?,
        ))
    }

    /// Start, stop, or restart a service.
    #[tool(
        description = "Start, stop, or restart a named OPNsense service. Call opnsense_list_services first to find valid service ids."
    )]
    pub async fn opnsense_service_control(
        &self,
        Parameters(ServiceControlArgs { name, action }): Parameters<ServiceControlArgs>,
    ) -> Result<Json<Value>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .service_control(&name, action.into())
                .await
                .map_err(opnsense_error)?,
        ))
    }

    /// Every network interface OPNsense knows about.
    #[tool(
        description = "List every network interface OPNsense knows about, keyed by device name."
    )]
    pub async fn opnsense_list_interfaces(&self) -> Result<Json<Value>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .list_interfaces()
                .await
                .map_err(opnsense_error)?,
        ))
    }

    /// Every configured firewall alias.
    #[tool(
        description = "List every firewall alias currently configured on OPNsense (name, type, and contents)."
    )]
    pub async fn opnsense_list_firewall_aliases(&self) -> Result<Json<Value>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .list_firewall_aliases()
                .await
                .map_err(opnsense_error)?,
        ))
    }

    /// Every configured gateway and its monitor status.
    #[tool(
        description = "List every configured gateway on OPNsense with its monitor status (none/loss/down) and ping latency."
    )]
    pub async fn opnsense_list_gateways(&self) -> Result<Json<Value>, ErrorData> {
        Ok(Json(
            self.opnsense()?
                .list_gateways()
                .await
                .map_err(opnsense_error)?,
        ))
    }
}

/// Turns a client-side failure into a protocol error the model can see and
/// reason about (bad service name, expired key, unreachable host, ...).
fn opnsense_error(err: rusty_opnsense::Error) -> ErrorData {
    ToolError::failed(err.to_string()).into()
}

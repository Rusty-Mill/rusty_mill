use std::time::Duration;

use rusty_request::Client;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::model::{Priority, ServiceAction, UnitType};

/// Where to find a `rusty_fedora_agent` instance.
///
/// The agent has no authentication of its own -- network reachability is
/// its only access control -- so `base_url` should always be a private/
/// Tailscale address, matching the agent's own `--bind` requirement.
#[derive(Debug, Clone)]
pub struct FedoraAgentConfig {
    /// The agent's base URL, e.g. `http://100.x.y.z:8765`. No trailing
    /// slash needed -- one is stripped if present.
    pub base_url: String,
    /// Per-request timeout. `None` uses `rusty_request`'s own default
    /// (30s) -- a `fedora_dnf_install`/`fedora_dnf_remove` call itself
    /// returns immediately (it hands back a task id), so this doesn't
    /// need to be long even though the underlying dnf run might be.
    pub timeout: Option<Duration>,
}

/// An async client for one `rusty_fedora_agent` instance's local HTTP
/// API.
///
/// Every method hands back the agent's own JSON response as-is, the same
/// passthrough style [`rusty_opnsense::OpnsenseClient`]/
/// [`rusty_proxmox::ProxmoxClient`] use against *their* upstream APIs --
/// `rusty_fedora_agent`'s response shapes are documented in its own
/// README rather than re-modeled here. Cheap to clone -- it shares the
/// same underlying `rusty_request::Client` (connection pool included).
#[derive(Debug, Clone)]
pub struct FedoraAgentClient {
    http: Client,
    base_url: String,
}

impl FedoraAgentClient {
    /// Build a client. Does not connect -- the first real request is
    /// whatever method is called first.
    pub fn new(config: FedoraAgentConfig) -> Self {
        let mut builder = Client::builder();
        if let Some(timeout) = config.timeout {
            builder = builder.timeout(timeout);
        }
        Self {
            http: builder.build(),
            base_url: config.base_url.trim_end_matches('/').to_string(),
        }
    }

    /// `GET /status` -- uptime, load average, memory, kernel/OS release.
    pub async fn system_status(&self) -> Result<Value> {
        self.get("/status").await
    }

    /// `GET /services[?unit_type=service|timer|socket]` -- every unit of
    /// the given type(s) with its load/active/sub state. `None` lists
    /// services, timers, and sockets together.
    pub async fn list_services(&self, unit_type: Option<UnitType>) -> Result<Value> {
        match unit_type {
            Some(unit_type) => self.get(&format!("/services?unit_type={unit_type}")).await,
            None => self.get("/services").await,
        }
    }

    /// `POST /services/{name}/control` -- start, stop, restart, enable,
    /// or disable a named unit (the short id from
    /// [`FedoraAgentClient::list_services`]). Refused by the agent if
    /// `name` isn't in its unit allowlist.
    pub async fn service_control(&self, name: &str, action: ServiceAction) -> Result<Value> {
        self.post_json(
            &format!("/services/{}/control", encode_path_segment(name)),
            &serde_json::json!({ "action": action.as_str() }),
        )
        .await
    }

    /// `GET /journal?unit=&lines=&since=&priority=` -- journal lines,
    /// most recent last. `unit: None` reads the full system journal;
    /// `lines: None` uses the agent's own default (100).
    pub async fn read_journal(
        &self,
        unit: Option<&str>,
        lines: Option<u32>,
        since: Option<&str>,
        priority: Option<Priority>,
    ) -> Result<Value> {
        let mut params = Vec::new();
        if let Some(unit) = unit {
            params.push(format!("unit={}", encode_query_value(unit)));
        }
        if let Some(lines) = lines {
            params.push(format!("lines={lines}"));
        }
        if let Some(since) = since {
            params.push(format!("since={}", encode_query_value(since)));
        }
        if let Some(priority) = priority {
            params.push(format!("priority={priority}"));
        }
        let query = if params.is_empty() {
            String::new()
        } else {
            format!("?{}", params.join("&"))
        };
        self.get(&format!("/journal{query}")).await
    }

    /// `GET /dnf/updates` -- every package with an update available.
    /// Call before [`FedoraAgentClient::dnf_install`].
    pub async fn dnf_list_updates(&self) -> Result<Value> {
        self.get("/dnf/updates").await
    }

    /// `POST /dnf/install` -- install `packages`. Runs asynchronously on
    /// the agent; returns a task id -- poll
    /// [`FedoraAgentClient::task_status`] with it. Refused by the agent
    /// if any package isn't in its package allowlist.
    pub async fn dnf_install(&self, packages: &[String]) -> Result<Value> {
        self.post_json("/dnf/install", &serde_json::json!({ "packages": packages }))
            .await
    }

    /// `POST /dnf/remove` -- remove `packages`. Same asynchronous-task
    /// shape as [`FedoraAgentClient::dnf_install`].
    pub async fn dnf_remove(&self, packages: &[String]) -> Result<Value> {
        self.post_json("/dnf/remove", &serde_json::json!({ "packages": packages }))
            .await
    }

    /// `GET /tasks/{id}` -- a dnf install/remove task's current state
    /// (`running`/`succeeded`/`failed`), plus stdout/stderr/exit_code once
    /// it's finished.
    pub async fn task_status(&self, task_id: &str) -> Result<Value> {
        self.get(&format!("/tasks/{}", encode_path_segment(task_id)))
            .await
    }

    /// `GET /config?path=...` -- one config file's raw content. Refused
    /// by the agent if `path` isn't under an allowlisted prefix.
    pub async fn read_config(&self, path: &str) -> Result<Value> {
        self.get(&format!("/config?path={}", encode_query_value(path)))
            .await
    }

    /// `PUT /config` -- replace one config file's content. `backup`
    /// (default `true` on the agent side if omitted here as `None`... --
    /// this client always sends it explicitly) writes a `.bak` copy of
    /// the previous content first, when the file already exists. Refused
    /// by the agent if `path` isn't under an allowlisted prefix.
    pub async fn write_config(&self, path: &str, content: &str, backup: bool) -> Result<Value> {
        self.put_json(
            "/config",
            &serde_json::json!({ "path": path, "content": content, "backup": backup }),
        )
        .await
    }

    async fn get(&self, path: &str) -> Result<Value> {
        let response = self
            .http
            .get(&format!("{}{path}", self.base_url))?
            .send()
            .await?;
        Self::parse(response).await
    }

    async fn post_json(&self, path: &str, payload: &Value) -> Result<Value> {
        let response = self
            .http
            .post(&format!("{}{path}", self.base_url))?
            .header("Content-Type", "application/json")?
            .body(serde_json::to_string(payload)?)
            .send()
            .await?;
        Self::parse(response).await
    }

    async fn put_json(&self, path: &str, payload: &Value) -> Result<Value> {
        let response = self
            .http
            .put(&format!("{}{path}", self.base_url))?
            .header("Content-Type", "application/json")?
            .body(serde_json::to_string(payload)?)
            .send()
            .await?;
        Self::parse(response).await
    }

    async fn parse(response: rusty_request::Response) -> Result<Value> {
        let status = response.status();
        let text = response.text()?;
        if status.is_client_error() || status.is_server_error() {
            return Err(Error::Api {
                status: status.as_u16(),
                body: text,
            });
        }
        Ok(serde_json::from_str(&text)?)
    }
}

/// Percent-encodes a value embedded directly in a path segment (a unit
/// name, a task id) -- narrower than [`encode_query_value`] since a path
/// segment must not contain a literal `/`.
fn encode_path_segment(value: &str) -> String {
    encode(value, |c| {
        c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~')
    })
}

/// Percent-encodes a query-string value, matching the decoding
/// `rusty_fedora_agent`'s own HTTP layer applies.
fn encode_query_value(value: &str) -> String {
    encode(value, |c| {
        c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~')
    })
}

fn encode(value: &str, is_safe: impl Fn(char) -> bool) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        let c = byte as char;
        if c.is_ascii() && is_safe(c) {
            out.push(c);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_segment_encoding_leaves_typical_unit_names_alone() {
        assert_eq!(encode_path_segment("ollama.service"), "ollama.service");
    }

    #[test]
    fn path_segment_encoding_escapes_a_slash() {
        assert_eq!(encode_path_segment("a/b"), "a%2Fb");
    }

    #[test]
    fn query_value_encoding_escapes_spaces_and_ampersands() {
        assert_eq!(encode_query_value("2 hours ago"), "2%20hours%20ago");
        assert_eq!(encode_query_value("a&b"), "a%26b");
    }
}

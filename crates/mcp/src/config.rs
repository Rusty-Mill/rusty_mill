//! `mcp.toml` parsing (PRD 07). Servers are declared with a transport and its
//! endpoint; the file is optional (a missing file means "no MCP servers").

use std::path::Path;

use serde::Deserialize;

use crate::McpError;

/// A configured MCP server transport.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    /// Local subprocess speaking JSON-RPC over stdio.
    Stdio,
    /// Remote streamable-HTTP / SSE endpoint.
    Sse,
}

/// One `[[servers]]` entry.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerSpec {
    /// Logical name; the namespace in `mcp__<name>__<tool>`.
    pub name: String,
    /// Transport kind.
    pub transport: Transport,
    /// stdio: the command to spawn.
    #[serde(default)]
    pub command: Option<String>,
    /// stdio: command arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// sse: the base URL.
    #[serde(default)]
    pub url: Option<String>,
    /// sse: env var holding the bearer token.
    #[serde(default)]
    pub auth_token_env: Option<String>,
}

/// The parsed `mcp.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct McpConfig {
    /// Declared servers.
    #[serde(default)]
    pub servers: Vec<ServerSpec>,
}

/// Load `mcp.toml` from `path`. A missing file yields an empty config (no MCP).
pub fn load_mcp_config(path: &Path) -> Result<McpConfig, McpError> {
    let body = match std::fs::read_to_string(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(McpConfig::default()),
        Err(e) => return Err(McpError::Transport(e.to_string())),
    };
    toml::from_str(&body).map_err(|e| McpError::Transport(format!("mcp.toml: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_empty() {
        let cfg = load_mcp_config(Path::new("/no/such/mcp.toml")).unwrap();
        assert!(cfg.servers.is_empty());
    }

    #[test]
    fn parses_stdio_and_sse_servers() {
        let dir = std::env::temp_dir().join(format!("rk-mcpcfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mcp.toml");
        std::fs::write(
            &path,
            r#"
[[servers]]
name = "filesystem"
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]

[[servers]]
name = "harness"
transport = "sse"
url = "https://mcp.harness.io/sse"
auth_token_env = "HARNESS_API_KEY"
"#,
        )
        .unwrap();
        let cfg = load_mcp_config(&path).unwrap();
        assert_eq!(cfg.servers.len(), 2);
        assert_eq!(cfg.servers[0].transport, Transport::Stdio);
        assert_eq!(cfg.servers[0].command.as_deref(), Some("npx"));
        assert_eq!(cfg.servers[1].transport, Transport::Sse);
        assert_eq!(
            cfg.servers[1].auth_token_env.as_deref(),
            Some("HARNESS_API_KEY")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! MCP server config parsing (PRD 07). Servers are declared with a transport
//! and its endpoint; configuration is optional (no config means "no MCP
//! servers").
//!
//! Servers are declared under the `[mcp]` table of the unified project config
//! file `<workspace>/.rustykeys/config.toml` (the P1 consolidation — see
//! `docs/assessment/RECOMMENDATIONS.md`):
//!
//! ```toml
//! [[mcp.servers]]
//! name = "filesystem"
//! transport = "stdio"
//! command = "npx"
//! args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
//! ```
//!
//! A legacy standalone `<workspace>/.rustykeys/mcp.toml` (top-level
//! `[[servers]]`) is still honored as a fallback for back-compat.

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

/// Load `mcp.toml` from `path` (legacy standalone file, top-level
/// `[[servers]]`). A missing file yields an empty config (no MCP).
pub fn load_mcp_config(path: &Path) -> Result<McpConfig, McpError> {
    let body = match std::fs::read_to_string(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(McpConfig::default()),
        Err(e) => return Err(McpError::Transport(e.to_string())),
    };
    toml::from_str(&body).map_err(|e| McpError::Transport(format!("mcp.toml: {e}")))
}

/// The `[mcp]` table of the unified project config file; other tables (e.g.
/// `[settings]`, owned by `rk-config`) are ignored so each crate parses only the
/// section it owns.
#[derive(Debug, Clone, Default, Deserialize)]
struct UnifiedFile {
    #[serde(default)]
    mcp: McpConfig,
}

/// Resolve MCP servers for a workspace (P1 consolidation). Reads the `[mcp]`
/// table of `<workspace>/.rustykeys/config.toml`; if it declares no servers,
/// falls back to a legacy standalone `<workspace>/.rustykeys/mcp.toml`. A
/// workspace with neither yields an empty config (no MCP).
pub fn load_mcp_config_for_workspace(workspace: &Path) -> Result<McpConfig, McpError> {
    let state_dir = workspace.join(".rustykeys");
    let unified_path = state_dir.join("config.toml");
    match std::fs::read_to_string(&unified_path) {
        Ok(body) => {
            let parsed: UnifiedFile = toml::from_str(&body)
                .map_err(|e| McpError::Transport(format!("config.toml: {e}")))?;
            if !parsed.mcp.servers.is_empty() {
                return Ok(parsed.mcp);
            }
            // Unified file present but declares no servers → try legacy file.
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(McpError::Transport(format!("config.toml: {e}"))),
    }
    load_mcp_config(&state_dir.join("mcp.toml"))
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

    fn workspace(tag: &str) -> std::path::PathBuf {
        let ws = std::env::temp_dir().join(format!("rk-mcpws-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(ws.join(".rustykeys")).unwrap();
        ws
    }

    #[test]
    fn reads_mcp_section_from_unified_config() {
        let ws = workspace("unified");
        std::fs::write(
            ws.join(".rustykeys").join("config.toml"),
            r#"
[settings]
model = "m"

[[mcp.servers]]
name = "filesystem"
transport = "stdio"
command = "npx"
"#,
        )
        .unwrap();
        let cfg = load_mcp_config_for_workspace(&ws).unwrap();
        assert_eq!(cfg.servers.len(), 1);
        assert_eq!(cfg.servers[0].name, "filesystem");
        assert_eq!(cfg.servers[0].transport, Transport::Stdio);
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn falls_back_to_legacy_mcp_toml() {
        let ws = workspace("legacy");
        // A unified file with no [mcp] table…
        std::fs::write(
            ws.join(".rustykeys").join("config.toml"),
            "[settings]\nmodel = \"m\"\n",
        )
        .unwrap();
        // …and a legacy standalone mcp.toml is still honored.
        std::fs::write(
            ws.join(".rustykeys").join("mcp.toml"),
            "[[servers]]\nname = \"legacy\"\ntransport = \"stdio\"\ncommand = \"cat\"\n",
        )
        .unwrap();
        let cfg = load_mcp_config_for_workspace(&ws).unwrap();
        assert_eq!(cfg.servers.len(), 1);
        assert_eq!(cfg.servers[0].name, "legacy");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn unified_servers_win_over_legacy() {
        let ws = workspace("both");
        std::fs::write(
            ws.join(".rustykeys").join("config.toml"),
            "[[mcp.servers]]\nname = \"unified\"\ntransport = \"stdio\"\ncommand = \"cat\"\n",
        )
        .unwrap();
        std::fs::write(
            ws.join(".rustykeys").join("mcp.toml"),
            "[[servers]]\nname = \"legacy\"\ntransport = \"stdio\"\ncommand = \"cat\"\n",
        )
        .unwrap();
        let cfg = load_mcp_config_for_workspace(&ws).unwrap();
        assert_eq!(cfg.servers.len(), 1);
        assert_eq!(cfg.servers[0].name, "unified");
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn no_config_yields_empty() {
        let ws = workspace("empty");
        let cfg = load_mcp_config_for_workspace(&ws).unwrap();
        assert!(cfg.servers.is_empty());
        let _ = std::fs::remove_dir_all(&ws);
    }
}

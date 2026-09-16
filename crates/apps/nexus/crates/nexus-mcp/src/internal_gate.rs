//! P1-02-style internal-only gate for the DG-39 dynamic-tool registry.
//!
//! `HANDLER_REGISTER_TOOL` (see [`crate::core_plugin`]) lets any plugin
//! publish an arbitrary caller-supplied `(plugin_id, command)` IPC route
//! as an MCP tool, and is itself `unrestricted` in `cap_matrix.toml` — no
//! capability is required to call it. When that dynamic tool is later
//! invoked, `NexusMcpServer::call_tool` (see [`crate::server`]) routes it
//! through the MCP server's own Core-trust, all-capabilities
//! `KernelPluginContext`, not the original registrant's. Without a check
//! here, a low-trust plugin could register a route into a handler
//! `cap_matrix.toml` marks `internal = true` — reachable, per
//! `nexus_kernel::context_impl::ipc_call_inner`'s `caller_trust_level`
//! gate, only from a Core-trust caller no matter what capabilities the
//! caller holds — and reach it anyway simply by naming it in an MCP
//! tool call. That is a confused-deputy bypass of the `internal = true`
//! gate.
//!
//! This module embeds the same `cap_matrix.toml` that
//! `nexus_bootstrap::cap_matrix::apply` treats as the single source of
//! truth. `nexus-bootstrap` depends on `nexus-mcp` (it wires
//! [`crate::McpHostPlugin`] into the plugin loader), not the other way
//! around, so this crate can't import `nexus_bootstrap::cap_matrix`
//! directly — it mirrors the minimal read-only shape of the file
//! instead and reads the identical bytes via `include_str!`.

use std::collections::HashSet;
use std::sync::LazyLock;

use serde::Deserialize;

/// Same embedded file `nexus_bootstrap::cap_matrix::apply` parses —
/// kept in sync by construction, since both read the identical TOML
/// off disk at compile time.
const MATRIX_TOML: &str = include_str!("../../nexus-bootstrap/cap_matrix.toml");

#[derive(Debug, Deserialize)]
struct MatrixFile {
    #[serde(default, rename = "handler")]
    handlers: Vec<RawHandler>,
}

/// Only the fields this module consumes; unlike
/// `nexus_bootstrap::cap_matrix::RawHandler` there is no
/// `deny_unknown_fields`-style strictness needed here — bootstrap's
/// own parser is the one responsible for validating the file's shape.
#[derive(Debug, Deserialize)]
struct RawHandler {
    plugin: String,
    command: String,
    #[serde(default)]
    internal: Option<bool>,
}

/// `(plugin_id, command)` pairs marked `internal = true`. Parsed once
/// and cached — the matrix is a compile-time constant, so there's
/// nothing to invalidate.
static INTERNAL_ONLY_PAIRS: LazyLock<HashSet<(String, String)>> = LazyLock::new(|| {
    let parsed: MatrixFile = toml::from_str(MATRIX_TOML)
        .expect("embedded cap_matrix.toml must parse (nexus-mcp mirrors nexus-bootstrap's copy)");
    parsed
        .handlers
        .into_iter()
        .filter(|h| h.internal == Some(true))
        .map(|h| (h.plugin, h.command))
        .collect()
});

/// True when `cap_matrix.toml` marks `(plugin_id, command)` as
/// `internal = true` — i.e. reachable only from a Core-trust caller.
///
/// A pair absent from the matrix entirely (e.g. a community/WASM
/// plugin's own command — `cap_matrix.toml` only governs in-tree core
/// plugin handlers) is not internal-only by definition, so this
/// returns `false` for it; that case is gated by the WASM sandbox's
/// own capability grants instead.
#[must_use]
pub(crate) fn is_internal_only(plugin_id: &str, command: &str) -> bool {
    INTERNAL_ONLY_PAIRS.contains(&(plugin_id.to_string(), command.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_the_known_internal_only_handler() {
        // `com.nexus.ai::resolve_credentials` — P1-02, returns provider
        // keyring material, gated to Core-trust callers only.
        assert!(is_internal_only("com.nexus.ai", "resolve_credentials"));
    }

    #[test]
    fn does_not_flag_an_unrestricted_handler() {
        assert!(!is_internal_only("com.nexus.mcp.host", "list_servers"));
    }

    #[test]
    fn does_not_flag_a_pair_absent_from_the_matrix() {
        assert!(!is_internal_only("com.example.plugin", "does_not_exist"));
    }
}

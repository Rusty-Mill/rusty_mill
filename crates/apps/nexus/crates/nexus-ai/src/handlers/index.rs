//! Indexing-related IPC handlers: `index_trigger`, `index_file`,
//! `vectorstore_count`, `status`.

use std::sync::{Arc, RwLock};

use nexus_kernel::{Ipc as _, KernelPluginContext};
use nexus_plugins::PluginError;
use tokio::sync::mpsc::UnboundedSender;

use crate::config::AiConfig;
use crate::handlers::shared::{
    build_embedding_provider, exec_err, resolve_embedding_dimension, resolve_embedding_model,
    tls_pinning_effective, PLUGIN_ID,
};
use crate::indexing_daemon::DaemonMsg;
use crate::{rag, vectorstore};

/// FU-2 — fan every markdown file in the storage index into the
/// indexing daemon as a `Touched`. Returns `{ queued: usize }`.
///
/// We go through `com.nexus.storage::query_files` rather than walking
/// the filesystem because storage already owns the canonical
/// inventory of forge files (and respects deletions, the
/// `.forge/` quarantine, etc.). Files not yet known to storage will
/// be picked up by the next file-watcher event — the indexing
/// daemon's debouncer dedupes overlap.
pub(crate) async fn handle_index_trigger(
    ctx: &KernelPluginContext,
    daemon_tx: &Arc<RwLock<Option<UnboundedSender<DaemonMsg>>>>,
) -> Result<serde_json::Value, PluginError> {
    let tx = daemon_tx
        .read()
        .ok()
        .and_then(|g| g.clone())
        .ok_or_else(|| exec_err("index_trigger: indexing daemon not running".to_string()))?;

    let response = ctx
        .ipc_call(
            "com.nexus.storage",
            "query_files",
            serde_json::json!({}),
            std::time::Duration::from_secs(30),
        )
        .await
        .map_err(|e| exec_err(format!("index_trigger: query_files: {e}")))?;

    let records = response
        .as_array()
        .ok_or_else(|| exec_err("index_trigger: query_files returned non-array".to_string()))?;

    let mut queued: usize = 0;
    for entry in records {
        let Some(path) = entry.get("path").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let ext_ok = std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"));
        if !ext_ok {
            continue;
        }
        if tx
            .send(DaemonMsg::Touched(std::path::PathBuf::from(path)))
            .is_ok()
        {
            queued += 1;
        }
    }

    tracing::debug!(plugin_id = PLUGIN_ID, queued, "index_trigger fanned forge");
    Ok(serde_json::json!({ "queued": queued }))
}

/// The `force` argument of `index_file`: omitted means `false`; present, it
/// must be a JSON boolean (`null`, strings and numbers are rejected rather
/// than guessed at).
fn parse_index_mode(args: &serde_json::Value) -> Result<rag::IndexMode, PluginError> {
    match args.get("force") {
        None | Some(serde_json::Value::Bool(false)) => Ok(rag::IndexMode::IfChanged),
        Some(serde_json::Value::Bool(true)) => Ok(rag::IndexMode::Force),
        Some(other) => Err(exec_err(format!(
            "index_file: 'force' must be a boolean, got {other}"
        ))),
    }
}

pub(crate) async fn handle_index_file(
    ctx: &KernelPluginContext,
    embed_cfg: Option<AiConfig>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, PluginError> {
    let file_path = args
        .get("file_path")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| exec_err("index_file: missing 'file_path' string".to_string()))?;
    let blocks: Vec<(u64, String, String, Option<i32>)> = args
        .get("blocks")
        .ok_or_else(|| exec_err("index_file: missing 'blocks'".to_string()))
        .and_then(|v| {
            serde_json::from_value(v.clone())
                .map_err(|e| exec_err(format!("index_file: blocks decode: {e}")))
        })?;

    let mode = parse_index_mode(args)?;

    let embed_cfg = embed_cfg
        .ok_or_else(|| exec_err("index_file: no AI embedding provider configured".to_string()))?;
    let embedder = build_embedding_provider(&embed_cfg).map_err(exec_err)?;

    let outcome = rag::index_file_with(ctx, embedder.as_ref(), file_path, &blocks, mode)
        .await
        .map_err(|e| exec_err(format!("index_file: {e}")))?;
    Ok(serde_json::json!({
        "indexed_chunks": outcome.chunks,
        "skipped": outcome.skipped,
    }))
}

pub(crate) async fn handle_vectorstore_count(
    ctx: &KernelPluginContext,
) -> Result<serde_json::Value, PluginError> {
    let count = vectorstore::count(ctx)
        .await
        .map_err(|e| exec_err(format!("vectorstore_count: {e}")))?;
    Ok(serde_json::json!({ "count": count }))
}

pub(crate) async fn handle_status(
    ctx: &KernelPluginContext,
    ai_cfg: Option<AiConfig>,
    embed_cfg: Option<AiConfig>,
) -> Result<serde_json::Value, PluginError> {
    let count = vectorstore::count(ctx)
        .await
        .map_err(|e| exec_err(format!("status: vectorstore_count: {e}")))?;
    let embedding_model = embed_cfg.as_ref().and_then(resolve_embedding_model);
    let embedding_dimension = embed_cfg.as_ref().and_then(resolve_embedding_dimension);
    let tls_pinned = tls_pinning_effective(ai_cfg.as_ref());
    let local_embeddings_supported = cfg!(feature = "local-embeddings");
    Ok(serde_json::json!({
        "ai_provider": ai_cfg.as_ref().map(|c| c.provider.clone()),
        "ai_model": ai_cfg.as_ref().and_then(|c| c.model.clone()),
        "embedding_provider": embed_cfg.as_ref().map(|c| c.provider.clone()),
        "embedding_model": embedding_model,
        "embedding_dimension": embedding_dimension,
        "indexed_chunks": count,
        "tls_pinned": tls_pinned,
        "local_embeddings_supported": local_embeddings_supported,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn omitted_force_means_false() {
        assert_eq!(
            parse_index_mode(&json!({ "file_path": "a.md" })).unwrap(),
            rag::IndexMode::IfChanged
        );
    }

    #[test]
    fn boolean_force_is_honoured() {
        assert_eq!(
            parse_index_mode(&json!({ "force": true })).unwrap(),
            rag::IndexMode::Force
        );
        assert_eq!(
            parse_index_mode(&json!({ "force": false })).unwrap(),
            rag::IndexMode::IfChanged
        );
    }

    #[test]
    fn malformed_force_is_rejected_not_guessed() {
        for bad in [
            json!(null),
            json!("true"),
            json!("false"),
            json!(1),
            json!(0),
            json!([]),
            json!({}),
        ] {
            let err = parse_index_mode(&json!({ "force": bad })).unwrap_err();
            assert!(
                err.to_string().contains("'force' must be a boolean"),
                "{bad}: {err}"
            );
        }
    }
}

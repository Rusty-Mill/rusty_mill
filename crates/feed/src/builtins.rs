//! Phase-1 built-in filesystem tools.
//!
//! The `#[tool]` macro produces the schema descriptor; the async `*_impl`
//! functions are the real bodies the adapter runs. Paths are resolved against
//! the workspace root so policy and execution agree on what a path means.

use std::path::{Path, PathBuf};

use crate::error::ToolError;
use crate::tool::{AiSdkTool, ToolRegistry};
use serde_json::Value;

mod descriptors {
    use aisdk::core::tools::Tool;
    use aisdk::macros::tool;

    #[tool(name = "read_file")]
    /// Read a UTF-8 file from the workspace. `path` is workspace-relative.
    pub fn read_file_descriptor(path: String) -> Tool {
        Ok(path)
    }

    #[tool(name = "list_directory")]
    /// List the entries of a directory in the workspace. `path` is workspace-relative.
    pub fn list_directory_descriptor(path: String) -> Tool {
        Ok(path)
    }
}

fn resolve(root: &Path, args: &Value) -> Result<PathBuf, ToolError> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::InvalidArgs("missing string field 'path'".into()))?;
    let p = Path::new(path);
    Ok(if p.is_absolute() { p.to_path_buf() } else { root.join(p) })
}

async fn read_file_impl(root: PathBuf, args: Value) -> Result<String, ToolError> {
    let path = resolve(&root, &args)?;
    tokio::fs::read_to_string(&path).await.map_err(|e| ToolError::Io(e.to_string()))
}

async fn list_directory_impl(root: PathBuf, args: Value) -> Result<String, ToolError> {
    let path = resolve(&root, &args)?;
    let mut entries =
        tokio::fs::read_dir(&path).await.map_err(|e| ToolError::Io(e.to_string()))?;
    let mut names = Vec::new();
    while let Some(entry) =
        entries.next_entry().await.map_err(|e| ToolError::Io(e.to_string()))?
    {
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(names.join("\n"))
}

/// Register the Phase-1 built-in filesystem tools, rooted at `workspace`.
pub fn register_builtins(registry: &mut ToolRegistry, workspace: PathBuf) {
    let root = workspace.clone();
    registry.insert(Box::new(AiSdkTool::new(
        descriptors::read_file_descriptor(),
        move |args| read_file_impl(root.clone(), args),
    )));
    let root = workspace;
    registry.insert(Box::new(AiSdkTool::new(
        descriptors::list_directory_descriptor(),
        move |args| list_directory_impl(root.clone(), args),
    )));
}

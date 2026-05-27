//! Built-in filesystem tools: read/list (Phase 1) + write/edit/glob/grep (Phase 6).
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

    #[tool(name = "write_file")]
    /// Create or overwrite a UTF-8 file (creates parent dirs). `path` is workspace-relative.
    pub fn write_file_descriptor(path: String, content: String) -> Tool {
        Ok(format!("{path}{content}"))
    }

    #[tool(name = "edit_file")]
    /// Replace `old_string` with `new_string` in a file; fails on 0 or 2+ matches.
    pub fn edit_file_descriptor(path: String, old_string: String, new_string: String) -> Tool {
        Ok(format!("{path}{old_string}{new_string}"))
    }

    #[tool(name = "glob")]
    /// List workspace files matching a glob `pattern` (e.g. `src/**/*.rs`).
    pub fn glob_descriptor(pattern: String) -> Tool {
        Ok(pattern)
    }

    #[tool(name = "grep")]
    /// Search file contents for a regex `pattern`; returns `path:line: text`, capped at 200.
    pub fn grep_descriptor(pattern: String, path: String) -> Tool {
        Ok(format!("{pattern}{path}"))
    }

    #[tool(name = "bash")]
    /// Run a shell `command` in the workspace (combined stdout+stderr). Vetted by
    /// BashGuard; default 30s timeout (override with `timeout_ms`).
    pub fn bash_descriptor(command: String) -> Tool {
        Ok(command)
    }
}

const GREP_CAP: usize = 200;
const BASH_DEFAULT_TIMEOUT_MS: u64 = 30_000;

fn resolve(root: &Path, args: &Value) -> Result<PathBuf, ToolError> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::InvalidArgs("missing string field 'path'".into()))?;
    let p = Path::new(path);
    Ok(if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    })
}

async fn read_file_impl(root: PathBuf, args: Value) -> Result<String, ToolError> {
    let path = resolve(&root, &args)?;
    Ok(tokio::fs::read_to_string(&path).await?)
}

async fn list_directory_impl(root: PathBuf, args: Value) -> Result<String, ToolError> {
    let path = resolve(&root, &args)?;
    let mut entries = tokio::fs::read_dir(&path).await?;
    let mut names = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(names.join("\n"))
}

fn arg_str(args: &Value, key: &str) -> Result<String, ToolError> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs(format!("missing string field '{key}'")))
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .into_owned()
}

async fn write_file_impl(root: PathBuf, args: Value) -> Result<String, ToolError> {
    let path = resolve(&root, &args)?;
    let content = arg_str(&args, "content")?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&path, content.as_bytes()).await?;
    Ok(format!(
        "wrote {} bytes to {}",
        content.len(),
        rel(&root, &path)
    ))
}

async fn edit_file_impl(root: PathBuf, args: Value) -> Result<String, ToolError> {
    let path = resolve(&root, &args)?;
    let old = arg_str(&args, "old_string")?;
    let new = arg_str(&args, "new_string")?;
    let content = tokio::fs::read_to_string(&path).await?;
    match content.matches(&old).count() {
        0 => Err(ToolError::InvalidArgs(format!(
            "edit_file: no match for old_string in {}",
            rel(&root, &path)
        ))),
        1 => {
            tokio::fs::write(&path, content.replacen(&old, &new, 1).as_bytes()).await?;
            Ok(format!("edited {}", rel(&root, &path)))
        }
        n => Err(ToolError::InvalidArgs(format!(
            "edit_file: old_string matches {n} times in {} (must be unique)",
            rel(&root, &path)
        ))),
    }
}

async fn glob_impl(root: PathBuf, args: Value) -> Result<String, ToolError> {
    let pattern = arg_str(&args, "pattern")?;
    let full = root.join(&pattern);
    let entries = glob::glob(&full.to_string_lossy())
        .map_err(|e| ToolError::InvalidArgs(format!("bad glob pattern: {e}")))?;
    let mut hits: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|p| rel(&root, &p))
        .collect();
    hits.sort();
    Ok(hits.join("\n"))
}

/// Recursively collect files under `dir`, skipping VCS/build/state noise.
fn collect_files(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "target" {
            continue;
        }
        if p.is_dir() {
            if recursive {
                collect_files(&p, recursive, out);
            }
        } else {
            out.push(p);
        }
    }
}

async fn grep_impl(root: PathBuf, args: Value) -> Result<String, ToolError> {
    let pattern = arg_str(&args, "pattern")?;
    let re = regex::Regex::new(&pattern)
        .map_err(|e| ToolError::InvalidArgs(format!("bad regex: {e}")))?;
    let start = match args.get("path").and_then(Value::as_str) {
        Some(p) if !p.is_empty() => resolve(&root, &args)?,
        _ => root.clone(),
    };
    let recursive = args
        .get("recursive")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let mut files = Vec::new();
    if start.is_dir() {
        collect_files(&start, recursive, &mut files);
    } else {
        files.push(start);
    }
    files.sort();

    let mut out = Vec::new();
    for file in files {
        let Ok(content) = std::fs::read_to_string(&file) else {
            continue;
        };
        for (i, line) in content.lines().enumerate() {
            if re.is_match(line) {
                out.push(format!(
                    "{}:{}: {}",
                    rel(&root, &file),
                    i + 1,
                    line.trim_end()
                ));
                if out.len() >= GREP_CAP {
                    out.push(format!("… (capped at {GREP_CAP} matches)"));
                    return Ok(out.join("\n"));
                }
            }
        }
    }
    Ok(out.join("\n"))
}

async fn bash_impl(root: PathBuf, args: Value) -> Result<String, ToolError> {
    let command = arg_str(&args, "command")?;
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(BASH_DEFAULT_TIMEOUT_MS);

    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg(&command).current_dir(&root);
    let run = cmd.output();

    match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), run).await {
        Err(_) => Err(ToolError::Timeout),
        Ok(Err(e)) => Err(e.into()),
        Ok(Ok(output)) => {
            let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
            combined.push_str(&String::from_utf8_lossy(&output.stderr));
            let combined = combined.trim_end().to_string();
            // A non-zero exit is *data*, not a tool error: the agent often runs
            // commands expecting failure (a failing test). Surface the code but
            // keep the outcome Ok so it doesn't spuriously fail verification.
            if output.status.success() {
                Ok(combined)
            } else {
                Ok(format!(
                    "[exit {}]\n{combined}",
                    output.status.code().unwrap_or(-1)
                ))
            }
        }
    }
}

/// Register the built-in filesystem tools, rooted at `workspace`.
pub fn register_builtins(registry: &mut ToolRegistry, workspace: PathBuf) {
    macro_rules! reg {
        ($desc:expr, $imp:ident) => {{
            let root = workspace.clone();
            registry.insert(Box::new(AiSdkTool::new($desc, move |args| {
                $imp(root.clone(), args)
            })));
        }};
    }
    reg!(descriptors::read_file_descriptor(), read_file_impl);
    reg!(
        descriptors::list_directory_descriptor(),
        list_directory_impl
    );
    reg!(descriptors::write_file_descriptor(), write_file_impl);
    reg!(descriptors::edit_file_descriptor(), edit_file_impl);
    reg!(descriptors::glob_descriptor(), glob_impl);
    reg!(descriptors::grep_descriptor(), grep_impl);
    reg!(descriptors::bash_descriptor(), bash_impl);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rk-builtins-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn write_then_read_round_trips_and_creates_dirs() {
        let root = tmp("write");
        write_file_impl(
            root.clone(),
            json!({"path": "sub/dir/a.txt", "content": "hello"}),
        )
        .await
        .unwrap();
        let got = read_file_impl(root.clone(), json!({"path": "sub/dir/a.txt"}))
            .await
            .unwrap();
        assert_eq!(got, "hello");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn edit_file_requires_a_unique_match() {
        let root = tmp("edit");
        write_file_impl(
            root.clone(),
            json!({"path": "f.txt", "content": "a X b X c"}),
        )
        .await
        .unwrap();
        // Two matches ⇒ rejected.
        let amb = edit_file_impl(
            root.clone(),
            json!({"path": "f.txt", "old_string": "X", "new_string": "Y"}),
        )
        .await;
        assert!(amb.is_err());
        // Unique match ⇒ replaced.
        edit_file_impl(
            root.clone(),
            json!({"path": "f.txt", "old_string": "a X", "new_string": "a Z"}),
        )
        .await
        .unwrap();
        let got = read_file_impl(root.clone(), json!({"path": "f.txt"}))
            .await
            .unwrap();
        assert_eq!(got, "a Z b X c");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn bash_runs_and_surfaces_nonzero_exit_as_data() {
        let root = tmp("bash");
        let ok = bash_impl(root.clone(), json!({"command": "echo hello"}))
            .await
            .unwrap();
        assert_eq!(ok, "hello");
        // Non-zero exit is surfaced (Ok), not a tool error.
        let nonzero = bash_impl(root.clone(), json!({"command": "exit 3"}))
            .await
            .unwrap();
        assert!(nonzero.starts_with("[exit 3]"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn glob_and_grep_find_content() {
        let root = tmp("glob");
        write_file_impl(
            root.clone(),
            json!({"path": "src/a.rs", "content": "fn main() {}\nlet x = 1;"}),
        )
        .await
        .unwrap();
        write_file_impl(
            root.clone(),
            json!({"path": "src/b.rs", "content": "fn helper() {}"}),
        )
        .await
        .unwrap();

        let globbed = glob_impl(root.clone(), json!({"pattern": "src/*.rs"}))
            .await
            .unwrap();
        assert!(globbed.contains("src/a.rs") && globbed.contains("src/b.rs"));

        let grepped = grep_impl(root.clone(), json!({"pattern": "fn \\w+", "path": ""}))
            .await
            .unwrap();
        assert!(grepped.contains("src/a.rs:1: fn main"));
        assert!(grepped.contains("src/b.rs:1: fn helper"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

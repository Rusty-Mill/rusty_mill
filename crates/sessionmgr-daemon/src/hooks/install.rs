//! Writes a session's own project-local hook config into its worktree.

use std::path::{Path, PathBuf};

use sessionmgr_core::{AgentKind, SessionId, SessionKind};

use crate::error::{Error, Result};

/// Installs `agent`'s hook config into `workspace_dir` (a worktree
/// session's own, isolated directory), pointing every hook event that
/// adapter cares about at `sessionmgr __hook-fire --session-id
/// <session_id> --event <name>`, invoked directly.
///
/// Requires `kind == Worktree`. Deliberately refused for
/// `SameDirectory` (that session's "workspace" is the user's own real
/// repository -- writing hook config there is a persistent side effect
/// on something this tool did not create and does not own, and it would
/// outlive the session) and `PlainTerminal` (no repository, nowhere to
/// write into). This is checked here, not left to the caller, so a
/// future call site cannot silently regress it.
pub fn install(
    kind: SessionKind,
    workspace_dir: &Path,
    agent: AgentKind,
    session_id: &SessionId,
) -> Result<PathBuf> {
    if kind != SessionKind::Worktree {
        return Err(Error::usage(
            "--hooks needs an isolated worktree session (--kind worktree); a \
             same-directory session's workspace is your own repository, and a \
             terminal session has no repository at all",
        ));
    }

    let exe =
        std::env::current_exe().map_err(|e| Error::io("locating this executable", None, e))?;
    // Forward-slashed, not the raw Windows path: measured directly (see
    // `AgentAdapterPort::hook_config`'s own docs) -- Claude Code
    // tokenizes a hook `command` string with POSIX-style backslash
    // escaping even on Windows, so `C:\a\b.exe` silently loses its
    // backslashes and becomes `C:ab.exe`.
    let exe = PathBuf::from(exe.to_string_lossy().replace('\\', "/"));

    let adapter = sessionmgr_agents::adapter_for(agent);
    let (relative_path, content) = adapter.hook_config(&exe, session_id);
    let full_path = workspace_dir.join(&relative_path);
    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            Error::io(
                "creating the hook config directory",
                parent.to_path_buf(),
                e,
            )
        })?;
    }
    std::fs::write(&full_path, content)
        .map_err(|e| Error::io("writing the hook config", full_path.clone(), e))?;
    Ok(full_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_same_directory_and_terminal_kinds() {
        let id = SessionId::new(1_700_000_000_000, 1);
        for kind in [SessionKind::SameDirectory, SessionKind::PlainTerminal] {
            let err = install(kind, Path::new("/does/not/matter"), AgentKind::Codex, &id)
                .expect_err("must refuse a non-worktree kind");
            assert!(matches!(err, Error::Usage { .. }));
        }
    }
}

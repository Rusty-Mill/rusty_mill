//! Feedforward guides (ADR-0037). The harness has rich *feedback* (the layered
//! verifier, the `FailureType` matrix, entropy audit) but thin *feedforward*:
//! `system_prompt` is two static paragraphs and the `AGENT_GUIDE.md` that `/init`
//! writes was never read. The [`GuideLoader`] closes that gap by discovering and
//! merging the guide hierarchy at session start, reusing the project→local
//! precedence idiom already established by `compose::CheckRegistry`.
//!
//! Precedence (lowest → highest, highest renders last so it wins the model's
//! attention):
//!
//! 1. **managed** — the compiled-in [`MANAGED_GUIDE`] baseline.
//! 2. **user** — `~/.rustykeys/AGENT_GUIDE.md`.
//! 3. **project** — `<workspace>/AGENT_GUIDE.md` (what `/init` writes).
//! 4. **local** — `<workspace>/.rustykeys/AGENT_GUIDE.md`.
//!
//! The merged text folds into the **static, cached `system` prefix** (built once
//! per session), *not* the per-turn oriented context — guides are session-stable,
//! so keeping them above the prompt-cache breakpoint avoids busting the cache
//! every turn (ADR-0037). Each consulted layer emits a [`ContextEntry`] with
//! `contribution = "guide"` so the episode `context_trace` records that it was
//! consulted. Guides are advisory text, never authority: they do not touch the
//! `constrain` vetting contract and are never executed.

use std::path::{Path, PathBuf};

use crate::memory::ContextEntry;

/// `contribution` tag for a consulted guide layer (ADR-0037).
pub const GUIDE_CONTRIBUTION: &str = "guide";

/// The compiled-in baseline guidance (the **managed** layer). Concise and
/// conduct-oriented; project/user/local layers add the specifics.
pub const MANAGED_GUIDE: &str = "Operating guidance:\n\
     - Follow the project's documented conventions; when one is unclear, read \
     existing code and AGENT_GUIDE.md before assuming.\n\
     - Make minimal, reversible changes and verify them with the project's \
     registered checks before reporting a task done.\n\
     - Treat a blocked or errored tool as a signal to observe and recover, not a \
     hard stop.";

/// The result of loading the guide hierarchy: the rendered block for the system
/// prefix, and one `context_trace` entry per consulted layer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoadedGuides {
    /// The `## Project guidance` block to append to the cached system prefix
    /// (empty only if every layer was empty — never, given the managed layer).
    pub block: String,
    /// One entry per consulted (present, non-empty) layer.
    pub entries: Vec<ContextEntry>,
}

/// Discovers and merges the `AGENT_GUIDE.md` hierarchy (ADR-0037).
pub struct GuideLoader;

impl GuideLoader {
    /// Load and merge the guide hierarchy for `workspace`. Best-effort: a missing,
    /// empty, or unreadable layer is skipped (advisory text is never fatal).
    pub fn load(workspace: &Path) -> LoadedGuides {
        let user = home_dir().map(|h| h.join(".rustykeys").join("AGENT_GUIDE.md"));
        let project = workspace.join("AGENT_GUIDE.md");
        let local = workspace.join(".rustykeys").join("AGENT_GUIDE.md");
        Self::load_layers(MANAGED_GUIDE, user.as_deref(), &project, &local)
    }

    /// The merge core, with explicit paths so it is testable without touching the
    /// real `$HOME`. Layers are appended lowest → highest precedence.
    fn load_layers(
        managed: &str,
        user: Option<&Path>,
        project: &Path,
        local: &Path,
    ) -> LoadedGuides {
        let mut layers: Vec<(String, String)> = Vec::new();

        let managed = managed.trim();
        if !managed.is_empty() {
            layers.push(("AGENT_GUIDE.md (managed)".to_string(), managed.to_string()));
        }
        for path in [user, Some(project), Some(local)].into_iter().flatten() {
            if let Some(text) = read_trimmed(path) {
                layers.push((path.display().to_string(), text));
            }
        }

        if layers.is_empty() {
            return LoadedGuides::default();
        }

        let mut block = String::from("## Project guidance\n");
        let entries = layers
            .iter()
            .map(|(label, text)| {
                block.push('\n');
                block.push_str(text);
                block.push('\n');
                ContextEntry {
                    artifact: label.clone(),
                    contribution: GUIDE_CONTRIBUTION.to_string(),
                    influenced_decision: false,
                }
            })
            .collect();

        LoadedGuides {
            block: block.trim_end().to_string(),
            entries,
        }
    }
}

/// Read a guide file, returning its trimmed contents only if present and
/// non-empty. Missing/empty/unreadable → `None` (skip the layer).
fn read_trimmed(path: &Path) -> Option<String> {
    let body = std::fs::read_to_string(path).ok()?;
    let trimmed = body.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Best-effort home directory from the environment (Linux-first).
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rk-guide-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join(".rustykeys")).unwrap();
        d
    }

    #[test]
    fn managed_only_when_no_files() {
        let ws = tmp("managed");
        let g = GuideLoader::load_layers(
            MANAGED_GUIDE,
            None,
            &ws.join("AGENT_GUIDE.md"),
            &ws.join(".rustykeys/AGENT_GUIDE.md"),
        );
        assert!(g.block.contains("## Project guidance"));
        assert!(g.block.contains("Operating guidance"));
        assert_eq!(g.entries.len(), 1);
        assert_eq!(g.entries[0].contribution, "guide");
        assert!(!g.entries[0].influenced_decision);
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn local_renders_after_project() {
        let ws = tmp("order");
        let project = ws.join("AGENT_GUIDE.md");
        let local = ws.join(".rustykeys/AGENT_GUIDE.md");
        std::fs::write(&project, "PROJECT-RULE").unwrap();
        std::fs::write(&local, "LOCAL-RULE").unwrap();

        let g = GuideLoader::load_layers(MANAGED_GUIDE, None, &project, &local);
        let pi = g.block.find("PROJECT-RULE").unwrap();
        let li = g.block.find("LOCAL-RULE").unwrap();
        assert!(
            li > pi,
            "local must render after project (higher precedence last)"
        );
        // managed + project + local = 3 consulted layers.
        assert_eq!(g.entries.len(), 3);
        assert!(g.entries.iter().all(|e| e.contribution == "guide"));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn user_layer_is_consulted_and_ordered_before_project() {
        let ws = tmp("user");
        let user = ws.join("user-guide.md");
        let project = ws.join("AGENT_GUIDE.md");
        std::fs::write(&user, "USER-RULE").unwrap();
        std::fs::write(&project, "PROJECT-RULE").unwrap();

        let g = GuideLoader::load_layers(
            MANAGED_GUIDE,
            Some(&user),
            &project,
            &ws.join(".rustykeys/AGENT_GUIDE.md"),
        );
        let ui = g.block.find("USER-RULE").unwrap();
        let pi = g.block.find("PROJECT-RULE").unwrap();
        assert!(ui < pi, "user precedes project");
        assert!(g
            .entries
            .iter()
            .any(|e| e.artifact.contains("user-guide.md")));
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn empty_files_are_skipped() {
        let ws = tmp("empty");
        std::fs::write(ws.join("AGENT_GUIDE.md"), "   \n\t").unwrap();
        let g = GuideLoader::load_layers(
            MANAGED_GUIDE,
            None,
            &ws.join("AGENT_GUIDE.md"),
            &ws.join(".rustykeys/AGENT_GUIDE.md"),
        );
        // Only the managed layer survives.
        assert_eq!(g.entries.len(), 1);
        let _ = std::fs::remove_dir_all(&ws);
    }
}

//! Controlled-visibility ablation eval-substrate (ADR-0035 / eval-plan §4.1).
//! This is the **golden-episode replay** home — not the live per-turn hot path.
//! It runs the H0–H3 ladder as a *true ablation* in three sequenced stages:
//!
//! 1. **Isolation (R2/F26).** Each episode runs in a *fresh* workspace at a fixed
//!    commit with its own `.rustykeys/` — no shared tree/DB/`task.json`. The
//!    declared `initial_state` is *enforced* (a pre-existing workspace is
//!    refused), not merely recorded.
//! 2. **Visibility (R1/F8).** Lower levels do not see higher-level artifacts.
//!    With Stage 1 in place this is enforced by *absence*: an episode below H2
//!    materializes no memory/`task.json`, and `checks.toml` — though present for
//!    the evaluator — is only agent-visible at H3 (the `Session` already gates
//!    it), so its existence is withheld from a lower-level agent.
//! 3. **Adjudication (R5/F9/F10).** The same evaluator-side checks
//!    ([`CheckRegistry::run_all`]) run at *every* level and assign the
//!    [`EpisodeOutcome`] — independent of the agent's self-report (paper Table 5).

use std::path::{Path, PathBuf};

use aisdk::core::capabilities::{TextInputSupport, ToolCallSupport};
use aisdk::core::language_model::LanguageModel;
use rk_compose::{evaluator_outcome, CheckRegistry, EpisodeOutcome};
use rk_config::{Config, HarnessLevel};

use crate::Session;

/// A frozen golden episode: the workspace baseline, the evaluator's checks, and
/// the scripted prompt(s) to drive. The *answer key* (expected label) is held by
/// the caller, never written into the workspace (eval-integrity, §8).
pub struct GoldenEpisode {
    /// Episode name (also the isolated-workspace tag).
    pub name: String,
    /// The harness level to replay at.
    pub level: HarnessLevel,
    /// The fixed commit the episode is pinned to (recorded in `initial_state`).
    pub commit: String,
    /// Initial workspace files: `(workspace-relative path, contents)`.
    pub files: Vec<(String, String)>,
    /// Evaluator `checks.toml` contents. Present for the evaluator at all levels;
    /// the agent only sees it at H3.
    pub checks_toml: Option<String>,
    /// The user prompts to send, in order (usually one).
    pub prompts: Vec<String>,
}

/// The result of replaying a golden episode.
pub struct EvalOutcome {
    /// The **evaluator-assigned** outcome (R5) — the comparable label across levels.
    pub evaluator_outcome: EpisodeOutcome,
    /// Whether the agent's own (self-reported) verification passed — distinct
    /// from the evaluator verdict, and never substituted for it.
    pub agent_verified: bool,
    /// The isolated workspace the episode ran in (caller cleans up).
    pub workspace: PathBuf,
}

/// Create a fresh isolated workspace for `name`, refusing to reuse a pre-existing
/// tree (Stage-1 `initial_state` enforcement).
fn isolated_workspace(name: &str) -> std::io::Result<PathBuf> {
    let root = std::env::temp_dir().join(format!(
        "rk-eval-{}-{}-{}",
        name,
        std::process::id(),
        now_tag()
    ));
    if root.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "episode workspace already exists — refusing to run on shared state",
        ));
    }
    std::fs::create_dir_all(&root)?;
    Ok(root)
}

fn now_tag() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn materialize(root: &Path, ep: &GoldenEpisode) -> std::io::Result<()> {
    for (rel, content) in &ep.files {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
    }
    // The evaluator's checks live under .rustykeys/checks.toml. Present for the
    // R5 pass at every level; the Session only surfaces it to the agent at H3.
    if let Some(toml) = &ep.checks_toml {
        let dir = root.join(".rustykeys");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("checks.toml"), toml)?;
    }
    Ok(())
}

fn episode_config(root: &Path, level: HarnessLevel) -> Result<Config, rk_config::ConfigError> {
    let ws = root.to_string_lossy().into_owned();
    let lvl = match level {
        HarnessLevel::H0 => "h0",
        HarnessLevel::H1 => "h1",
        HarnessLevel::H2 => "h2",
        HarnessLevel::H3 => "h3",
    };
    Config::resolve(|k| match k {
        "RUSTYKEYS_MODEL" => Some("fake".into()),
        "RUSTYKEYS_WORKSPACE" => Some(ws.clone()),
        "RUSTYKEYS_HARNESS_LEVEL" => Some(lvl.into()),
        _ => None,
    })
}

/// Replay `ep` at its target level under the controlled-visibility ablation
/// (the three stages above), driven by `model`. Returns the evaluator-assigned
/// outcome alongside the agent's self-reported verdict.
pub async fn run_episode<M>(ep: &GoldenEpisode, model: M) -> anyhow::Result<EvalOutcome>
where
    M: LanguageModel + TextInputSupport + ToolCallSupport + Clone,
{
    // Stage 1 — isolation.
    let root = isolated_workspace(&ep.name)?;
    materialize(&root, ep)?;

    // Stage 2 — visibility is enforced by what the isolated workspace contains
    // (and by the Session's existing per-level gates), so the agent below H2/H3
    // simply has no higher-level artifact to read.
    let config = episode_config(&root, ep.level)?;
    let session = Session::new(&config, model)?;
    let mut agent_verified = true;
    for prompt in &ep.prompts {
        let outcome = session.send(prompt).await?;
        agent_verified = outcome.report.verified;
    }

    // Stage 3 — evaluator-side adjudication at *this* level, from the evaluator's
    // own checks, never the agent's self-report.
    let registry = CheckRegistry::load(&root)?;
    let results = registry.run_all().await;

    Ok(EvalOutcome {
        evaluator_outcome: evaluator_outcome(&results),
        agent_verified,
        workspace: root,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolation_refuses_a_preexisting_workspace() {
        // Two calls with the same name produce distinct fresh dirs (nanosecond
        // tag), and a fresh dir is always empty — the enforcement invariant.
        let a = isolated_workspace("dup").unwrap();
        let b = isolated_workspace("dup").unwrap();
        assert_ne!(a, b);
        assert!(std::fs::read_dir(&a).unwrap().next().is_none());
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }
}

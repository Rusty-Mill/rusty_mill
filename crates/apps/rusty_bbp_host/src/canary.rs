//! `bbp canary` (#599 PR 4, D217): one profile of a task's frozen profile set
//! re-run on a released ref, as a report and nothing more.
//!
//! The maintenance watcher (`rusty_orch watch`) calls it for each app: REF is
//! resolved to a full commit, the named profile runs on a clean sandboxed
//! checkout of it through the #598 baseline runner
//! ([`crate::runner::isolated_baseline`]), and the result is printed as a
//! [`Baseline`]. Nothing is written to the task: a red canary becomes a claim
//! only through the watcher's `watch_finding` post. The task must be open
//! (the human opens each maintenance task, D80), since its frozen set and
//! holdout suite are read from it. No model is called, so a canary costs
//! nothing.

use crate::baseline::{self, Baseline};
use crate::profiles::{self, ProfileSet};
use crate::{holdout, open_driver, runner};
use rusty_bbp::TaskId;
use rusty_sandbox::Executor;
use std::path::Path;

/// `set` cut down to the one profile named `profile`.
///
/// # Errors
///
/// A message naming the profile when the set has none by that name.
pub fn only(set: &ProfileSet, profile: &str) -> Result<ProfileSet, String> {
    let mut one = set.clone();
    one.profiles.retain(|p| p.name == profile);
    if one.profiles.is_empty() {
        let names: Vec<&str> = set.profiles.iter().map(|p| p.name.as_str()).collect();
        return Err(format!(
            "no profile {profile:?} in the task's profile set (it has {})",
            names.join(", ")
        ));
    }
    Ok(one)
}

/// Run `task`'s frozen `profile` on `reference` in `repo`, under
/// `work_root/baseline`, with `exec`.
///
/// # Errors
///
/// A message when the task, its profile set or holdout suite can't be read,
/// the profile isn't in the set, REF doesn't resolve, or the run is refused
/// because the holdout suite could be read from it.
pub fn run<E: Executor>(
    exec: &E,
    dir: &Path,
    task: &TaskId,
    repo: &Path,
    work_root: &Path,
    reference: &str,
    profile: &str,
) -> Result<Baseline, String>
where
    E::Error: std::fmt::Display,
{
    let (set, holdout) = {
        let d = open_driver(dir, task)?;
        let set = profiles::load(dir, task, d.state.profile_digest)?;
        let holdout = d
            .state
            .claims
            .holdout_digest
            .map(|h| holdout::load(dir, task, h))
            .transpose()?;
        (set, holdout)
    };
    let one = only(&set, profile)?;
    let base = baseline::resolve(repo, reference)?;
    runner::isolated_baseline(exec, &one, holdout.as_ref(), repo, work_root, &base)
        .map_err(|r| format!("{:?}: {}", r.code, r.detail))
}

/// [`run`] in the host sandbox, as one line of JSON.
///
/// # Errors
///
/// As [`run`], and when the host can't sandbox.
pub fn json(
    dir: &Path,
    task: &TaskId,
    repo: &Path,
    work_root: &Path,
    reference: &str,
    profile: &str,
) -> Result<String, String> {
    runner::probe_host()?;
    let exec = runner::sandboxed(&dir.join("sandbox-state"))?;
    let b = run(&exec, dir, task, repo, work_root, reference, profile)?;
    rusty_serde::json::to_string(&b).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_keeps_the_named_profile() {
        let mut set = ProfileSet::shell("unit", "true");
        let mut other = set.profiles[0].clone();
        other.name = "e2e".into();
        set.profiles.push(other);
        let one = only(&set, "e2e").expect("found");
        assert_eq!(one.profiles.len(), 1);
        assert_eq!(one.profiles[0].name, "e2e");
        assert_eq!(one.env, set.env, "the rest of the set is kept");
        let err = only(&set, "lint").expect_err("missing");
        assert!(
            err.contains("\"lint\"") && err.contains("unit, e2e"),
            "{err}"
        );
    }
}

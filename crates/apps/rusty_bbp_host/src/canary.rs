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
//!
//! The caller's own deadline must outlast the workload: the sandbox runs in
//! its own process group, and only this process's wall timer kills it, so a
//! caller that kills `bbp canary` first would orphan the workload. With
//! `max_wall_secs` the run is refused, before anything is checked out, when
//! the set's wall limit is longer than the caller is willing to wait.

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

/// Refuse a set whose wall limit is longer than `max_wall_secs`.
///
/// # Errors
///
/// A message naming both limits.
pub fn within(set: &ProfileSet, max_wall_secs: Option<u64>) -> Result<(), String> {
    match max_wall_secs {
        Some(max) if set.limits.wall_secs > max => Err(format!(
            "the profile set's wall limit ({}s) is longer than --max-wall-secs ({max}s)",
            set.limits.wall_secs
        )),
        _ => Ok(()),
    }
}

/// What to run: `task`'s frozen `profile` on `reference` in `repo`, under
/// `work_root/baseline`, refused when its wall limit is over `max_wall_secs`.
pub struct Request<'a> {
    pub dir: &'a Path,
    pub task: &'a TaskId,
    pub repo: &'a Path,
    pub work_root: &'a Path,
    pub reference: &'a str,
    pub profile: &'a str,
    pub max_wall_secs: Option<u64>,
}

/// Run `r` with `exec`.
///
/// # Errors
///
/// A message when the task, its profile set or holdout suite can't be read,
/// the profile isn't in the set, its wall limit is too long, REF doesn't
/// resolve, or the run is refused because the holdout suite could be read
/// from it.
pub fn run<E: Executor>(exec: &E, r: &Request<'_>) -> Result<Baseline, String>
where
    E::Error: std::fmt::Display,
{
    let (dir, task) = (r.dir, r.task);
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
    let one = only(&set, r.profile)?;
    within(&one, r.max_wall_secs)?;
    let base = baseline::resolve(r.repo, r.reference)?;
    runner::isolated_baseline(exec, &one, holdout.as_ref(), r.repo, r.work_root, &base)
        .map_err(|e| format!("{:?}: {}", e.code, e.detail))
}

/// [`run`] in the host sandbox, as one line of JSON.
///
/// # Errors
///
/// As [`run`], and when the host can't sandbox.
pub fn json(r: &Request<'_>) -> Result<String, String> {
    runner::probe_host()?;
    let exec = runner::sandboxed(&r.dir.join("sandbox-state"))?;
    let b = run(&exec, r)?;
    rusty_serde::json::to_string(&b).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
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

    #[test]
    fn a_wall_limit_past_the_callers_wait_is_refused() {
        let mut set = ProfileSet::shell("unit", "true");
        set.limits.wall_secs = 600;
        assert_eq!(within(&set, None), Ok(()));
        assert_eq!(within(&set, Some(600)), Ok(()));
        let err = within(&set, Some(599)).expect_err("too long");
        assert!(err.contains("600s") && err.contains("599s"), "{err}");
    }
}

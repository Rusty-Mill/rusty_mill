//! `bbp canary` (#599 PR 4, D217): one profile of a task's frozen profile set
//! re-run on a released ref, as a report and nothing more.
//!
//! The maintenance watcher (`rusty_orch watch`) calls it for each app: REF is
//! resolved to a full commit, the named profile runs on a clean sandboxed
//! checkout of it through the #598 baseline runner
//! ([`crate::runner::isolated_baseline_gated`]), and the result is printed as
//! a [`Baseline`]. Nothing is written to the task: a red canary becomes a
//! claim only through the watcher's `watch_finding` post. The task must be
//! open (the human opens each maintenance task, D80), since its frozen set
//! and holdout suite are read from it. No model is called, so a canary costs
//! nothing.
//!
//! The sandbox runs in its own process group and only this process's wall
//! timer kills it, so a caller that kills `bbp canary` before that timer
//! fires would orphan the workload. With a [`Budget`] (`--budget-secs N`, counted from this
//! process's start) the run is refused when the budget can't cover the wall
//! limit plus [`CLEANUP`]: once up front, and again after all setup (the
//! lock wait, checkout, test listing and scratch directory), immediately
//! before the sandbox launches. A refusal starts nothing and is the report's
//! `error`.
//!
//! What this guarantees, on the supported Linux path (the only one the host
//! sandbox runs on) under normal scheduling: a run that starts sends
//! `SIGKILL` to the whole sandbox group at its wall limit, at least
//! [`CLEANUP`] before the budget ends. `SIGKILL` can't be caught and the
//! sandbox's seccomp filter denies `setsid`/`setpgid` (`rusty_sandbox`
//! `helper::group_lock`), so once it is sent no workload process survives,
//! even if the caller then kills this process. Reaping and the report are
//! outside the guarantee: [`CLEANUP`] is a reserve for them, not a bound. If
//! they outlast it, the caller loses the report and records a timeout, but
//! nothing is orphaned.

use crate::baseline::{self, Baseline};
use crate::profiles::{self, ProfileSet};
use crate::{holdout, open_driver, runner};
use rusty_bbp::TaskId;
use rusty_sandbox::Executor;
use std::path::Path;
use std::time::{Duration, Instant};

/// Time between the sandbox's group kill and the end of the budget, for
/// reaping and the report. A reserve only: neither is bounded by it.
pub const CLEANUP: Duration = Duration::from_secs(60);

/// How long the caller will wait for this run, from `started`.
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    pub started: Instant,
    pub total: Duration,
}

impl Budget {
    /// `Ok` while what is left still covers `wall` and [`CLEANUP`].
    ///
    /// # Errors
    ///
    /// A message giving the time left and the time the run needs.
    pub fn covers(&self, wall: Duration) -> Result<(), String> {
        let left = self.total.saturating_sub(self.started.elapsed());
        let need = wall + CLEANUP;
        if left < need {
            return Err(format!(
                "budget: {}s left, the run needs {}s (the wall limit plus {}s cleanup); nothing started",
                left.as_secs(),
                need.as_secs(),
                CLEANUP.as_secs()
            ));
        }
        Ok(())
    }
}

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

/// What to run: `task`'s frozen `profile` on `reference` in `repo`, under
/// `work_root/baseline`, within `budget` when one is given.
pub struct Request<'a> {
    pub dir: &'a Path,
    pub task: &'a TaskId,
    pub repo: &'a Path,
    pub work_root: &'a Path,
    pub reference: &'a str,
    pub profile: &'a str,
    pub budget: Option<Budget>,
}

/// Run `r` with `exec`.
///
/// # Errors
///
/// A message when the task, its profile set or holdout suite can't be read,
/// the profile isn't in the set, the budget can't cover it from the start,
/// REF doesn't resolve, or the run is refused because the holdout suite
/// could be read from it. A budget that runs short during setup is the
/// report's `error` instead.
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
    let wall = Duration::from_secs(one.limits.wall_secs.max(1));
    let ready = || r.budget.map_or(Ok(()), |b| b.covers(wall));
    ready()?;
    let base = baseline::resolve(r.repo, r.reference)?;
    runner::isolated_baseline_gated(
        exec,
        &one,
        holdout.as_ref(),
        r.repo,
        r.work_root,
        &base,
        &ready,
    )
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
    fn a_budget_covers_the_wall_limit_and_cleanup_or_refuses() {
        let wall = Duration::from_secs(600);
        let fresh = Budget {
            started: Instant::now(),
            total: Duration::from_secs(3600),
        };
        assert_eq!(fresh.covers(wall), Ok(()));
        let tight = Budget {
            total: wall + CLEANUP - Duration::from_secs(1),
            ..fresh
        };
        let err = tight.covers(wall).expect_err("too tight");
        assert!(
            err.contains("660s") && err.contains("nothing started"),
            "{err}"
        );
        // Time already spent counts against it.
        let spent = Budget {
            started: Instant::now()
                .checked_sub(Duration::from_secs(3000))
                .expect("clock"),
            ..fresh
        };
        assert!(spent.covers(wall).is_err(), "600s left < 660s needed");
    }
}

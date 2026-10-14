//! `bbp metrics`: the concurrency metrics of one task, read from its log.

use crate::open_driver;
use rusty_bbp::metrics::{self, Metrics, Overlap};
use rusty_bbp::spec_metrics::{self, SpecAttribution, SpecConflicts};
use rusty_bbp::*;
use rusty_serde::Serialize;
use std::path::Path;

/// What `bbp metrics` prints. Wall clock is not here: the log carries no
/// timestamps; `bbp mod` reports `elapsed_ms` for the run.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Report {
    pub task: String,
    pub max_live_turns: u32,
    pub state: String,
    pub metrics: Metrics,
    pub overlap: Overlap,
    /// Each non-approved spec version against the approved one (2b criterion 4).
    pub spec_conflicts: SpecConflicts,
    /// Whether each non-approved spec version's work reached the approved one.
    pub spec_attribution: SpecAttribution,
    /// Versions that are both a `Conflict` and `Unused` (criterion 4's count).
    pub unused_conflicts: u64,
}

/// Read `task`'s log and blobs and compute its [`Report`].
pub fn of(dir: &Path, task: &TaskId) -> Result<Report, String> {
    let d = open_driver(dir, task)?;
    let events = d.store.events(task, 0).map_err(|e| format!("{e:?}"))?;
    let blob = |b: &BlobRef| d.store.blob_get(&b.sha).ok();
    let overlap = metrics::overlap(&events, &blob);
    let spec_conflicts = spec_metrics::spec_conflicts(&events, &blob);
    let spec_attribution = spec_metrics::spec_attribution(&events, &blob);
    let unused_conflicts = spec_metrics::unused_conflicts(&spec_conflicts, &spec_attribution);
    Ok(Report {
        task: task.0.clone(),
        max_live_turns: d.state.max_live_turns,
        state: format!("{:?}", d.state.state),
        metrics: metrics::of(&events),
        overlap,
        spec_conflicts,
        spec_attribution,
        unused_conflicts,
    })
}

/// [`of`] as one line of JSON.
pub fn json(dir: &Path, task: &TaskId) -> Result<String, String> {
    rusty_serde::json::to_string(&of(dir, task)?).map_err(|e| e.to_string())
}

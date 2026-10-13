//! `bbp metrics`: the concurrency metrics of one task, read from its log.

use crate::open_driver;
use rusty_bbp::metrics::{self, Metrics, Overlap};
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
}

/// Read `task`'s log and blobs and compute its [`Report`].
pub fn of(dir: &Path, task: &TaskId) -> Result<Report, String> {
    let d = open_driver(dir, task)?;
    let events = d.store.events(task, 0).map_err(|e| format!("{e:?}"))?;
    let overlap = metrics::overlap(&events, &|b| d.store.blob_get(&b.sha).ok());
    Ok(Report {
        task: task.0.clone(),
        max_live_turns: d.state.max_live_turns,
        state: format!("{:?}", d.state.state),
        metrics: metrics::of(&events),
        overlap,
    })
}

/// [`of`] as one line of JSON.
pub fn json(dir: &Path, task: &TaskId) -> Result<String, String> {
    rusty_serde::json::to_string(&of(dir, task)?).map_err(|e| e.to_string())
}

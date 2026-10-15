//! The background GitHub sync: one thread that runs [`reconcile`] every so often, and can be
//! started, stopped and restarted while the board serves (a settings change replaces it).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::api::Shared;
use crate::github::{GithubConfig, GithubForge};
use crate::sync::reconcile;

/// Shortest interval between runs, in seconds.
pub const MIN_SECS: u64 = 5;

struct Running {
    stop: Arc<AtomicBool>,
    repo: String,
}

/// Owns the sync thread, if one is running.
pub struct Syncer {
    board: Arc<Shared>,
    running: Mutex<Option<Running>>,
    /// Held while a run is in progress, so a replaced thread's last run and the new thread's
    /// first never overlap.
    run: Arc<Mutex<()>>,
}

impl Syncer {
    /// A syncer for `board`, not yet running.
    pub fn new(board: Arc<Shared>) -> Self {
        Self {
            board,
            running: Mutex::new(None),
            run: Arc::new(Mutex::new(())),
        }
    }

    /// Start syncing with `config` every `every`, replacing any running sync.
    ///
    /// # Errors
    /// If the HTTP client or the thread cannot start (an existing sync keeps running).
    pub fn start(&self, config: GithubConfig, repo: &str, every: Duration) -> Result<(), String> {
        let forge = GithubForge::new(config).map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let (flag, board, run) = (
            Arc::clone(&stop),
            Arc::clone(&self.board),
            Arc::clone(&self.run),
        );
        std::thread::Builder::new()
            .name("github-sync".into())
            .spawn(move || {
                while !flag.load(Ordering::SeqCst) {
                    {
                        let _one_at_a_time = run.lock().unwrap_or_else(PoisonError::into_inner);
                        if !flag.load(Ordering::SeqCst) {
                            match reconcile(&board, &forge) {
                                Ok(r)
                                    if r.is_quiet()
                                        && r.drift.is_empty()
                                        && r.missing.is_empty() => {}
                                Ok(r) => tracing::info!(?r, "github sync"),
                                Err(e) => {
                                    tracing::warn!(error = %e, "github sync failed; will retry")
                                }
                            }
                        }
                    }
                    // Sleep in short steps so a stop (or a replacement) takes effect promptly.
                    let mut left = every;
                    while !left.is_zero() && !flag.load(Ordering::SeqCst) {
                        let step = left.min(Duration::from_millis(500));
                        std::thread::sleep(step);
                        left -= step;
                    }
                }
            })
            .map_err(|e| format!("cannot start sync thread: {e}"))?;
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(old) = running.replace(Running {
            stop,
            repo: repo.to_owned(),
        }) {
            old.stop.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    /// Stop syncing (a run in progress finishes first).
    pub fn stop(&self) {
        if let Some(old) = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            old.stop.store(true, Ordering::SeqCst);
        }
    }

    /// The repository being synced, if a sync is running.
    pub fn repo(&self) -> Option<String> {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|r| r.repo.clone())
    }
}

impl Drop for Syncer {
    fn drop(&mut self) {
        self.stop();
    }
}

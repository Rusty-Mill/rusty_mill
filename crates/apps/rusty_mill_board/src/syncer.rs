//! The background GitHub sync: one thread that runs [`reconcile`] every so often, and can be
//! started, stopped and restarted while the board serves (a settings change replaces it).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::api::Shared;
use crate::github::{GithubConfig, GithubForge};
use crate::mergewatch::watch;
use crate::sync::reconcile;

/// Shortest interval between runs, in seconds.
pub const MIN_SECS: u64 = 5;

/// Shortest interval between merge watch runs: it reads whole pages of closed pull requests
/// with the sync's token, so it runs far less often than the sync itself.
const WATCH_EVERY: Duration = Duration::from_secs(300);

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

    /// Start syncing with `config` every `every`, replacing any running sync. With `auto_close`
    /// the merge watch also closes cards a merged PR named (`Closes card #N`); without, it
    /// only lists them.
    ///
    /// # Errors
    /// If the HTTP client or the thread cannot start (an existing sync keeps running).
    pub fn start(
        &self,
        config: GithubConfig,
        repo: &str,
        every: Duration,
        auto_close: bool,
    ) -> Result<(), String> {
        let pulls = config.merged()?;
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
                let mut last_watch: Option<Instant> = None;
                while !flag.load(Ordering::SeqCst) {
                    {
                        let _one_at_a_time = run.lock().unwrap_or_else(PoisonError::into_inner);
                        if !flag.load(Ordering::SeqCst) {
                            if last_watch.is_none_or(|t| t.elapsed() >= WATCH_EVERY) {
                                last_watch = Some(Instant::now());
                                if let Err(e) = watch(&board, &pulls, auto_close) {
                                    rusty_log::warn!(error = %e, "merge watch failed; will retry");
                                }
                            }
                            match reconcile(&board, &forge) {
                                Ok(r)
                                    if r.is_quiet()
                                        && r.drift.is_empty()
                                        && r.missing.is_empty() => {}
                                Ok(r) => rusty_log::info!(?r, "github sync"),
                                Err(e) => {
                                    rusty_log::warn!(error = %e, "github sync failed; will retry")
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

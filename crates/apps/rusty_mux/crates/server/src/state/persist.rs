//! Sessions that survive a restart (C45): restoring the store at start,
//! and saving what changed every [`SAVE_EVERY`] and on a clean stop.
//!
//! A restored block's program died with the old server. A shell block gets
//! a fresh copy of the configured program in its last working directory,
//! under its saved screen and a one-line marker. A block started with a
//! command of its own comes back read-only, showing its saved screen until
//! closed: running the command again could repeat what it did (a build, a
//! deploy, an agent's turn). Owner's pick, rusty_term D72.

use std::collections::HashMap;
use std::path::Path;
use std::rc::Weak;
use std::time::Duration;

use rusty_mux_core::{BlockId, Mux, SessionId, Size, WindowId};
use rusty_term_core::{AnsiParser, Grid};

use super::{Shared, Start, State};
use crate::store::{Loaded, Manifest, Started, Store};
use crate::terminal;

/// How often a running server saves what changed: what a crash can lose.
pub(crate) const SAVE_EVERY: Duration = Duration::from_secs(5);

/// One save, taken from the state at once and written off its thread.
pub(crate) struct Snapshot {
    store: Store,
    generation: u64,
    manifest: Manifest,
    screens: Vec<(BlockId, Vec<u8>)>,
}

impl Snapshot {
    fn write(&self) -> std::io::Result<()> {
        self.store
            .save(self.generation, &self.manifest, &self.screens)
    }
}

/// Where a restored block goes, and how it was started before.
struct Revive {
    block: BlockId,
    window: WindowId,
    session: SessionId,
    size: Size,
    before: Option<Started>,
    screen: Option<(Grid, AnsiParser)>,
}

impl State {
    /// Opens the store `Config::state_dir` names and restores what it
    /// holds, before any client connects. Nothing here stops the start: a
    /// store that cannot be opened saves nothing, and one that cannot be
    /// read is moved aside.
    pub(crate) fn restore(&mut self, shared: &Shared) {
        let Some(dir) = self.config.state_dir.clone() else {
            return;
        };
        let store = match Store::open(dir) {
            Ok(store) => store,
            Err(e) => return eprintln!("rusty-mux-server: sessions will not be saved: {e}"),
        };
        let loaded = store.load().unwrap_or_else(|why| set_aside(&store, &why));
        self.store = Some(store.clone());
        let Some(Loaded { manifest, screens }) = loaded else {
            return;
        };
        let mux = match Mux::restore(manifest.saved) {
            Ok(mux) => mux,
            Err(why) => return drop(set_aside(&store, &why.to_string())),
        };
        self.mux = mux;
        let mut screens: HashMap<_, _> = screens.into_iter().map(|(b, g, p)| (b, (g, p))).collect();
        let mut started = manifest.started;
        for revive in self.revivals(&mut started, &mut screens) {
            let block = revive.block;
            if let Err(why) = self.revive(shared, revive) {
                eprintln!(
                    "rusty-mux-server: block {block} not restored: {}",
                    why.reason
                );
                let _ = self.mux.close_block(block);
            }
        }
        self.flush();
    }

    fn revivals(
        &self,
        started: &mut std::collections::BTreeMap<BlockId, Started>,
        screens: &mut HashMap<BlockId, (Grid, AnsiParser)>,
    ) -> Vec<Revive> {
        let mut out = Vec::new();
        for session in self.mux.sessions() {
            for window in session.windows().iter().filter_map(|w| self.mux.window(*w)) {
                for block in window.blocks().iter().filter_map(|b| self.mux.block(*b)) {
                    out.push(Revive {
                        block: block.id(),
                        window: window.id(),
                        session: session.id(),
                        size: block.size(),
                        before: started.remove(&block.id()),
                        screen: screens.remove(&block.id()),
                    });
                }
            }
        }
        out
    }

    /// Brings `revive.block` back: a shell block with a fresh copy of the
    /// configured program, a command block read-only (D72). Either shows
    /// its saved screen under a marker, written once per block.
    fn revive(&mut self, shared: &Shared, revive: Revive) -> Result<(), super::Refusal> {
        crate::checkpoint::validate_size(revive.size)?;
        let (cols, rows) = (revive.size.cols().into(), revive.size.rows().into());
        let (mut grid, mut parser) = revive.screen.unwrap_or_else(|| {
            super::fresh_screen(Grid::new(cols, rows), self.config.grapheme_clustering)
        });
        if (grid.cols, grid.rows) != (cols, rows) {
            grid.resize(cols, rows);
        }
        let before = revive.before;
        let already = before.as_ref().is_some_and(|b| b.read_only);
        if !already {
            // What the marker scrolls out of a full scrollback goes to the
            // spool (C70), which `adopt` opens.
            grid.spill_evicted(self.store.is_some());
            mark(&mut grid, &mut parser, before.as_ref());
            // Queries in the saved screen were answered in the old run.
            drop(parser.take_responses());
        }
        let cwd = before.as_ref().map(|b| b.cwd.clone());
        if let Some(command) = before.filter(|b| !b.shell) {
            let started = Started {
                read_only: true,
                ..command
            };
            let rt = super::BlockRt::read_only(grid, parser, started);
            self.adopt(revive.block, rt);
            return Ok(());
        }
        let config = self.config_in(cwd.as_deref());
        let started = Started {
            shell: true,
            read_only: false,
            program: config.program.to_string_lossy().into_owned(),
            args: config
                .args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect(),
            cwd: config.cwd.to_string_lossy().into_owned(),
        };
        let unscreened = self.start(Start {
            block: revive.block,
            session: revive.session,
            window: revive.window,
            size: revive.size,
            config: &config,
            started,
            // A restored block has no owner (D117 (7)), and was never told
            // a caller's environment (D74).
            owned: false,
            env: None,
            fds: Vec::new(),
        })?;
        let rt = unscreened.screened(shared, grid, parser);
        self.adopt(revive.block, rt);
        Ok(())
    }

    /// The configured program, in `cwd` when that is an absolute directory.
    fn config_in(&self, cwd: Option<&str>) -> crate::Config {
        let mut config = self.config.clone();
        if let Some(cwd) = cwd
            && Path::new(cwd).is_absolute()
            && Path::new(cwd).is_dir()
        {
            config.cwd = cwd.into();
        }
        config
    }

    /// What changed since the last save, or `None` when nothing did, no
    /// store is open, or the server is stopping (its blocks are closing,
    /// which is not a change to save).
    pub(crate) fn snapshot(&mut self) -> Option<Snapshot> {
        if self.stopping {
            return None;
        }
        let store = self.store.clone()?;
        let seq = self.history.cursor().seq;
        let mut screens = Vec::new();
        for (block, rt) in &mut self.blocks {
            if !rt.unsaved {
                continue;
            }
            match rusty_term_core::checkpoint::encode(&rt.grid, &rt.parser) {
                Ok(data) => screens.push((*block, data)),
                Err(e) => eprintln!("rusty-mux-server: saving block {block}'s screen: {e}"),
            }
            rt.unsaved = false;
        }
        if screens.is_empty() && self.saved_at == Some(seq) {
            return None;
        }
        self.saved_at = Some(seq);
        self.generation += 1;
        let started = self
            .blocks
            .iter()
            .map(|(block, rt)| {
                let cwd =
                    terminal::pwd(&rt.grid, rt.pid()).unwrap_or_else(|| rt.started.cwd.clone());
                (
                    *block,
                    Started {
                        cwd,
                        ..rt.started.clone()
                    },
                )
            })
            .collect();
        let manifest = Manifest {
            saved: self.mux.save(),
            started,
        };
        Some(Snapshot {
            store,
            generation: self.generation,
            manifest,
            screens,
        })
    }

    /// A save failed, or the next must be whole: it writes everything.
    fn save_failed(&mut self) {
        self.saved_at = None;
        self.blocks.values_mut().for_each(|rt| rt.unsaved = true);
    }

    /// Saves everything now, on this thread: for a clean stop. Whole, so
    /// a periodic save still in flight is older and skipped.
    pub(crate) fn save_now(&mut self) {
        self.save_failed();
        let Some(snapshot) = self.snapshot() else {
            return;
        };
        if let Err(e) = snapshot.write() {
            eprintln!("rusty-mux-server: saving sessions: {e}");
        }
    }
}

/// Saves what changed every [`SAVE_EVERY`], writing on the blocking pool,
/// one save at a time; ends with the state.
pub(crate) async fn save_periodically(state: Weak<std::cell::RefCell<State>>) {
    loop {
        rusty_tokio::time::sleep(SAVE_EVERY).await;
        let Some(shared) = state.upgrade() else {
            return;
        };
        let snapshot = shared.borrow_mut().snapshot();
        drop(shared);
        let Some(snapshot) = snapshot else {
            continue;
        };
        let written = rusty_tokio::spawn_blocking(move || snapshot.write())
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))
            .and_then(|r| r);
        if let Err(e) = written {
            eprintln!("rusty-mux-server: saving sessions: {e}");
            if let Some(shared) = state.upgrade() {
                shared.borrow_mut().save_failed();
            }
        }
    }
}

/// Moves an unreadable store aside, saying where; restores nothing.
fn set_aside(store: &Store, why: &str) -> Option<Loaded> {
    match store.set_aside() {
        Ok(aside) => eprintln!(
            "rusty-mux-server: saved sessions unreadable ({why}); moved to {} and starting empty",
            aside.display()
        ),
        Err(e) => eprintln!(
            "rusty-mux-server: saved sessions unreadable ({why}) and not moved aside: {e}"
        ),
    }
    None
}

/// Writes [`marker`] under a restored screen, keeping all its text: cancels
/// any sequence the old program left unfinished, leaves the alternate
/// screen with its text copied to the main one (which, unlike it, keeps
/// what scrolls off), resets modes (DECSTR) and starts below the last row,
/// wherever the saved cursor was.
fn mark(grid: &mut Grid, parser: &mut AnsiParser, before: Option<&Started>) {
    let full_screen = terminal::Capture {
        history: 0,
        trim: true,
        unwrap: false,
        vt: true,
    };
    let alternate = grid
        .in_alt_screen()
        .then(|| terminal::format(grid, full_screen, usize::MAX))
        .flatten();
    let mut out = format!("\x18\x1b[?1049l\x1b[!p\x1b[{};1H\r\n", grid.rows);
    for line in alternate.iter().flat_map(|text| text.split('\n')) {
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str(&marker(before));
    parser.advance(grid, out.as_bytes());
}

/// What happened, dimmed, on a line of its own.
fn marker(before: Option<&Started>) -> String {
    let note = match before {
        Some(b) if !b.shell => {
            let command = std::iter::once(&b.program)
                .chain(&b.args)
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            let command: String = command.chars().filter(|c| !c.is_control()).collect();
            format!("restored after a server restart, read-only: `{command}` was not run again")
        }
        _ => "restored after a server restart".into(),
    };
    format!("\x1b[2m[{note}]\x1b[0m\r\n")
}

#[cfg(test)]
mod tests;

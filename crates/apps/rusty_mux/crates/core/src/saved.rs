//! What of the model outlives a server run (C45): sessions, windows, their
//! layouts and blocks, by id. Clients, leases and queued events belong to
//! one run and are left out.
//!
//! The tree nests, so a window can only be in one session and a block in
//! one window; [`Mux::restore`] checks the rest.

use std::collections::BTreeSet;
use std::fmt;

use crate::{
    Block, BlockId, Layout, MAX_DEPTH, MAX_NAME, Mux, Session, SessionId, Size, Window, WindowId,
};

/// Every session, in creation order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Saved {
    pub sessions: Vec<SavedSession>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedSession {
    pub id: SessionId,
    pub name: String,
    pub generated: bool,
    /// One of `windows`; `None` only when there are none.
    pub active: Option<WindowId>,
    /// In creation order.
    pub windows: Vec<SavedWindow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedWindow {
    pub id: WindowId,
    pub name: String,
    /// Holds `blocks` but the detached ones.
    pub layout: Option<Layout>,
    pub zoomed: Option<BlockId>,
    pub focused: Option<BlockId>,
    /// In creation order.
    pub blocks: Vec<SavedBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedBlock {
    pub id: BlockId,
    pub name: String,
    pub size: Size,
}

/// A [`Saved`] tree that breaks one of the model's rules; nothing is
/// restored from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestoreError(pub &'static str);

impl fmt::Display for RestoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "saved state: {}", self.0)
    }
}

impl std::error::Error for RestoreError {}

impl Mux {
    /// The part of the model a later run restores.
    pub fn save(&self) -> Saved {
        let sessions = self
            .sessions()
            .map(|s| SavedSession {
                id: s.id,
                name: s.name.clone(),
                generated: s.generated,
                active: s.active,
                windows: s
                    .windows
                    .iter()
                    .filter_map(|w| self.window(*w))
                    .map(|w| self.save_window(w))
                    .collect(),
            })
            .collect();
        Saved { sessions }
    }

    fn save_window(&self, w: &Window) -> SavedWindow {
        SavedWindow {
            id: w.id,
            name: w.name.clone(),
            layout: w.layout.clone(),
            zoomed: w.zoomed,
            focused: w.focused,
            blocks: w
                .blocks
                .iter()
                .filter_map(|b| self.block(*b))
                .map(|b| SavedBlock {
                    id: b.id,
                    name: b.name.clone(),
                    size: b.size,
                })
                .collect(),
        }
    }

    /// A model holding `saved` and nothing else: no clients, no leases and
    /// no queued events, since clients of the earlier run resync as on any
    /// new run. Refuses a tree that breaks a rule the operations keep.
    pub fn restore(saved: Saved) -> Result<Self, RestoreError> {
        check(&saved)?;
        let mut mux = Self::new();
        for s in saved.sessions {
            mux.session_order.push(s.id);
            let windows = s.windows.iter().map(|w| w.id).collect();
            for w in s.windows {
                mux.restore_window(s.id, w);
            }
            let session = Session {
                id: s.id,
                name: s.name,
                windows,
                active: s.active,
                generated: s.generated,
            };
            mux.sessions.insert(s.id, session);
        }
        Ok(mux)
    }

    fn restore_window(&mut self, session: SessionId, w: SavedWindow) {
        let blocks = w.blocks.iter().map(|b| b.id).collect();
        for b in w.blocks {
            let block = Block {
                id: b.id,
                window: w.id,
                name: b.name,
                size: b.size,
                lease: None,
                owner: None,
            };
            self.blocks.insert(b.id, block);
        }
        let window = Window {
            id: w.id,
            session,
            name: w.name,
            blocks,
            layout: w.layout,
            zoomed: w.zoomed,
            focused: w.focused,
        };
        self.windows.insert(w.id, window);
    }
}

/// Every rule [`Mux::restore`] relies on, before anything is built.
fn check(saved: &Saved) -> Result<(), RestoreError> {
    let mut ids = BTreeSet::new();
    let mut fresh = |id: rusty_uuid::Uuid| ids.insert(id).then_some(());
    for s in &saved.sessions {
        fresh(s.id.uuid()).ok_or(RestoreError("an id appears twice"))?;
        long(&s.name)?;
        let owns = |w: WindowId| s.windows.iter().any(|x| x.id == w);
        match s.active {
            None if !s.windows.is_empty() => return Err(RestoreError("no active window")),
            Some(w) if !owns(w) => return Err(RestoreError("active window not in session")),
            _ => {}
        }
        for w in &s.windows {
            fresh(w.id.uuid()).ok_or(RestoreError("an id appears twice"))?;
            for b in &w.blocks {
                fresh(b.id.uuid()).ok_or(RestoreError("an id appears twice"))?;
            }
            check_window(w)?;
        }
    }
    Ok(())
}

fn check_window(w: &SavedWindow) -> Result<(), RestoreError> {
    long(&w.name)?;
    w.blocks.iter().try_for_each(|b| long(&b.name))?;
    let placed = w.layout.as_ref().map(Layout::blocks).unwrap_or_default();
    let owns = |b: &BlockId| w.blocks.iter().any(|x| x.id == *b);
    if !placed.iter().all(owns) {
        return Err(RestoreError("layout names a block of another window"));
    }
    let mut unique = placed.clone();
    unique.sort_unstable();
    unique.dedup();
    let deep = |b: &BlockId| {
        w.layout
            .as_ref()
            .and_then(|l| l.depth_of(*b))
            .is_none_or(|d| d > MAX_DEPTH)
    };
    if unique.len() != placed.len() || placed.iter().any(deep) {
        return Err(RestoreError("layout repeats a block or nests too deep"));
    }
    let in_layout = |b: Option<BlockId>| b.is_none_or(|b| placed.contains(&b));
    if !in_layout(w.zoomed) || !in_layout(w.focused) {
        return Err(RestoreError("zoomed or focused block not in layout"));
    }
    if w.focused.is_none() != placed.is_empty() {
        return Err(RestoreError("focus does not match layout"));
    }
    if w.zoomed.is_some_and(|z| w.focused != Some(z)) {
        return Err(RestoreError("zoomed block is not the focused one"));
    }
    Ok(())
}

fn long(name: &str) -> Result<(), RestoreError> {
    if name.len() > MAX_NAME {
        return Err(RestoreError("name too long"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;

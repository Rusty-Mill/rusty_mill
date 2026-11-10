//! The model's state and the operations that change it.

use std::collections::BTreeMap;

use crate::names;
use crate::{
    Axis, Block, BlockId, Client, ClientId, EntityId, Error, Event, ExitStatus, InputMode, Layout,
    MAX_DEPTH, Owner, Principal, Progress, Selection, Session, SessionId, Share, Side, Size,
    Toward, Transport, Window, WindowId,
};

/// The longest name any entity may have, in UTF-8 bytes. Names travel in
/// every event and listing, so a bound keeps one name from filling a
/// client's queue or a listing's reply.
pub const MAX_NAME: usize = 256;

/// The longest a grant may run, in ms: 24 hours (C77).
pub const MAX_GRANT_MS: u64 = 24 * 60 * 60 * 1000;

/// Refuses a name longer than [`MAX_NAME`].
fn check_name(name: &str) -> Result<(), Error> {
    if name.len() > MAX_NAME {
        return Err(Error::NameTooLong { len: name.len() });
    }
    Ok(())
}

/// All sessions, windows, blocks and clients, plus the events not yet drained.
///
/// Every operation either succeeds and queues its events, or fails and
/// changes nothing.
#[derive(Debug, Default)]
pub struct Mux {
    pub(crate) sessions: BTreeMap<SessionId, Session>,
    /// Session ids in creation order.
    pub(crate) session_order: Vec<SessionId>,
    pub(crate) windows: BTreeMap<WindowId, Window>,
    pub(crate) blocks: BTreeMap<BlockId, Block>,
    clients: BTreeMap<ClientId, Client>,
    events: Vec<Event>,
}

impl Mux {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the queued events, oldest first, and clears the queue.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    pub fn session(&self, id: SessionId) -> Option<&Session> {
        self.sessions.get(&id)
    }

    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.get(&id)
    }

    pub fn block(&self, id: BlockId) -> Option<&Block> {
        self.blocks.get(&id)
    }

    pub fn client(&self, id: ClientId) -> Option<&Client> {
        self.clients.get(&id)
    }

    /// Sessions in creation order.
    pub fn sessions(&self) -> impl Iterator<Item = &Session> {
        self.session_order
            .iter()
            .filter_map(|id| self.sessions.get(id))
    }

    pub fn create_session(&mut self, name: impl Into<String>) -> Result<SessionId, Error> {
        self.add_session(name.into(), false)
    }

    fn add_session(&mut self, name: String, generated: bool) -> Result<SessionId, Error> {
        check_name(&name)?;
        let id = SessionId::random();
        let session = Session {
            id,
            name,
            windows: Vec::new(),
            active: None,
            generated,
            creator: None,
        };
        self.sessions.insert(id, session);
        self.session_order.push(id);
        self.events.push(Event::SessionCreated { session: id });
        Ok(id)
    }

    /// Records `creator` as the principal that created `session`, which
    /// decides who may disconnect the clients attached to it (D164).
    pub fn set_creator(&mut self, session: SessionId, creator: Principal) -> Result<(), Error> {
        let session = self
            .sessions
            .get_mut(&session)
            .ok_or(Error::UnknownSession(session))?;
        session.creator = Some(creator);
        Ok(())
    }

    /// The session `client` is attached to, through its block's window.
    pub fn session_of_client(&self, client: ClientId) -> Option<&Session> {
        let block = self.clients.get(&client)?.attached?;
        let window = self.blocks.get(&block)?.window;
        let session = self.windows.get(&window)?.session;
        self.sessions.get(&session)
    }

    /// Creates a session with a generated name, such as `quiet-harbor`,
    /// that no other session has; `seed` picks it ([`is_generated`]
    /// recognises its shape). The caller supplies the randomness, so the model stays
    /// deterministic.
    ///
    /// [`is_generated`]: crate::is_generated
    pub fn create_generated_session(&mut self, seed: u64) -> Result<SessionId, Error> {
        let name = names::generate(seed, |name| self.sessions.values().any(|s| s.name == name));
        self.add_session(name, true)
    }

    pub fn create_window(
        &mut self,
        session: SessionId,
        name: impl Into<String>,
    ) -> Result<WindowId, Error> {
        let name = name.into();
        check_name(&name)?;
        let parent = self
            .sessions
            .get_mut(&session)
            .ok_or(Error::UnknownSession(session))?;
        let id = WindowId::random();
        parent.windows.push(id);
        // Only a session's first window becomes active by itself, so a
        // window an agent opens in the background takes nobody's view.
        let first = parent.active.is_none();
        if first {
            parent.active = Some(id);
        }
        self.windows.insert(
            id,
            Window {
                id,
                session,
                name,
                blocks: Vec::new(),
                layout: None,
                zoomed: None,
                focused: None,
            },
        );
        self.events.push(Event::WindowCreated {
            window: id,
            session,
        });
        if first {
            self.events.push(Event::ActiveWindowChanged {
                session,
                window: Some(id),
            });
        }
        Ok(id)
    }

    /// Makes `window` its session's active window; selecting the active
    /// one again changes nothing.
    pub fn select_window(&mut self, window: WindowId) -> Result<(), Error> {
        let session = self
            .windows
            .get(&window)
            .ok_or(Error::UnknownWindow(window))?
            .session;
        let parent = self
            .sessions
            .get_mut(&session)
            .ok_or(Error::UnknownSession(session))?;
        if parent.active == Some(window) {
            return Ok(());
        }
        parent.active = Some(window);
        self.events.push(Event::ActiveWindowChanged {
            session,
            window: Some(window),
        });
        Ok(())
    }

    pub fn create_block(&mut self, window: WindowId, size: Size) -> Result<BlockId, Error> {
        let id = BlockId::random();
        self.add_block(id, window, size)?;
        Ok(id)
    }

    /// Adds a block whose id the caller chose, such as one already given to
    /// the block's program before it started. It splits the window's
    /// largest block that may still be split, side by side if that block is
    /// wider than tall (a cell being about twice as tall as wide), stacked
    /// otherwise; a zoom stays.
    pub fn add_block(&mut self, id: BlockId, window: WindowId, size: Size) -> Result<(), Error> {
        if self.blocks.contains_key(&id) {
            return Err(Error::BlockExists(id));
        }
        if !self.windows.contains_key(&window) {
            return Err(Error::UnknownWindow(window));
        }
        let layout = match self.layout_of(window).cloned() {
            None => Layout::Block(id),
            Some(layout) => {
                let target = layout.largest().ok_or(Error::TooDeep(id))?;
                let side = match self.blocks.get(&target).map(|b| b.size) {
                    Some(s) if s.cols / 2 < s.rows => Side::Down,
                    _ => Side::Right,
                };
                layout.split(target, id, side)
            }
        };
        if let Some(parent) = self.windows.get_mut(&window) {
            parent.layout = Some(layout);
        }
        self.insert_block(id, window, size);
        Ok(())
    }

    /// The size a block made by splitting `target` toward `side` gets: its
    /// part of `target`'s size.
    pub fn split_size(&self, target: BlockId, side: Side) -> Result<Size, Error> {
        let size = self.split_target(target)?.size;
        let pick = |len: u16| {
            let (first, second) = Share::HALF.lengths(len).ok_or(Error::NoRoom {
                block: target,
                side,
            })?;
            Ok(if side.first() { first } else { second })
        };
        Ok(match side.axis() {
            Axis::Columns => Size {
                cols: pick(size.cols)?,
                rows: size.rows,
            },
            Axis::Rows => Size {
                cols: size.cols,
                rows: pick(size.rows)?,
            },
        })
    }

    /// Adds a block whose id the caller chose beside `target`, splitting
    /// `target`'s place in half, and unzooms the window so it shows. The
    /// size is the caller's, usually [`Self::split_size`]; `target` keeps
    /// its own until a client resizes it.
    pub fn split_block(
        &mut self,
        id: BlockId,
        target: BlockId,
        side: Side,
        size: Size,
    ) -> Result<(), Error> {
        if self.blocks.contains_key(&id) {
            return Err(Error::BlockExists(id));
        }
        let window = self.split_target(target)?.window;
        let parent = self
            .windows
            .get_mut(&window)
            .ok_or(Error::UnknownWindow(window))?;
        parent.layout = parent.layout.take().map(|l| l.split(target, id, side));
        parent.zoomed = None;
        self.insert_block(id, window, size);
        Ok(())
    }

    /// The block `target`, if it exists and may be split.
    fn split_target(&self, target: BlockId) -> Result<&Block, Error> {
        let block = self
            .blocks
            .get(&target)
            .ok_or(Error::UnknownBlock(target))?;
        let depth = self
            .windows
            .get(&block.window)
            .and_then(|w| w.layout.as_ref())
            .and_then(|l| l.depth_of(target))
            .ok_or(Error::UnknownBlock(target))?;
        if depth >= MAX_DEPTH {
            return Err(Error::TooDeep(target));
        }
        Ok(block)
    }

    /// Records a block already placed in its window's layout.
    fn insert_block(&mut self, id: BlockId, window: WindowId, size: Size) {
        if let Some(parent) = self.windows.get_mut(&window) {
            parent.blocks.push(id);
        }
        self.blocks.insert(
            id,
            Block {
                id,
                window,
                name: String::new(),
                size,
                lease: None,
                owner: None,
                input: InputMode::Open,
                grants: BTreeMap::new(),
            },
        );
        self.events.push(Event::BlockCreated {
            block: id,
            window,
            size,
        });
        self.push_layout(window, None);
    }

    /// Shows `block` alone over its whole window, until it is unzoomed,
    /// closed, or another block is split off in the window. A detached
    /// block is refused: only a block in the layout can be shown.
    pub fn zoom(&mut self, block: BlockId) -> Result<(), Error> {
        let window = self.window_of(block)?;
        if !self.layout_of(window).is_some_and(|l| l.contains(block)) {
            return Err(Error::Detached(block));
        }
        self.set_zoom(window, Some(block))
    }

    /// Shows every block of the window again.
    pub fn unzoom(&mut self, window: WindowId) -> Result<(), Error> {
        self.set_zoom(window, None)
    }

    fn set_zoom(&mut self, window: WindowId, zoomed: Option<BlockId>) -> Result<(), Error> {
        let target = self
            .windows
            .get_mut(&window)
            .ok_or(Error::UnknownWindow(window))?;
        if target.zoomed == zoomed {
            return Ok(());
        }
        target.zoomed = zoomed;
        self.push_layout(window, None);
        Ok(())
    }

    /// Gives `block` `share` of the innermost split along `axis` that holds
    /// it, so a client can resize the split between it and its neighbour.
    pub fn resize_split(&mut self, block: BlockId, axis: Axis, share: Share) -> Result<(), Error> {
        let window = self.window_of(block)?;
        let mut layout = self
            .layout_of(window)
            .filter(|l| l.contains(block))
            .cloned()
            .ok_or(Error::Detached(block))?;
        if !layout.resize(block, axis, share) {
            return Err(Error::NoSplit { block, axis });
        }
        self.set_layout(window, layout, false, None);
        Ok(())
    }

    /// Puts each block where the other was, in the window they share. If
    /// one is detached, the other is detached in its place. A zoom stays
    /// on its block, or moves to the block taking its place if it left.
    pub fn swap_blocks(&mut self, block: BlockId, other: BlockId) -> Result<(), Error> {
        let window = self.same_window(block, other)?;
        let Some(mut layout) = self.layout_of(window).cloned() else {
            return Ok(());
        };
        layout.swap(block, other);
        let partner = |b: BlockId| if b == block { other } else { block };
        let mut heir = None;
        if let Some(w) = self.windows.get_mut(&window) {
            if let Some(zoomed) = w.zoomed.filter(|z| !layout.contains(*z)) {
                w.zoomed = Some(partner(zoomed));
            }
            heir = w.focused.map(partner);
        }
        self.set_layout(window, layout, false, heir);
        Ok(())
    }

    /// Takes `block` from its place, if it has one, and splits `beside`'s
    /// place in half to put it on `side`, unzooming the window as a split
    /// does. Without `beside`, `block` fills the window, which must show
    /// no block. Either way a detached block comes back.
    pub fn move_block(
        &mut self,
        block: BlockId,
        beside: Option<BlockId>,
        side: Side,
    ) -> Result<(), Error> {
        let window = match beside {
            Some(target) if target == block => return Err(Error::SameBlock(block)),
            Some(target) => self.same_window(block, target)?,
            None => self.window_of(block)?,
        };
        let layout = match (self.layout_of(window).cloned(), beside) {
            (None, None) => Layout::Block(block),
            (Some(_), None) => return Err(Error::Placed(window)),
            (None, Some(target)) => return Err(Error::Detached(target)),
            (Some(layout), Some(target)) => {
                let rest = layout.remove(block).ok_or(Error::Detached(target))?;
                let depth = rest.depth_of(target).ok_or(Error::Detached(target))?;
                if depth >= MAX_DEPTH {
                    return Err(Error::TooDeep(target));
                }
                rest.split(target, block, side)
            }
        };
        self.set_layout(window, layout, true, None);
        Ok(())
    }

    /// Takes `block` out of its window's layout, its sibling taking its
    /// split's place; it keeps running, listed in the window, until moved
    /// back in or closed. A zoom on it ends. A detached block stays so.
    pub fn detach_block(&mut self, block: BlockId) -> Result<(), Error> {
        let window = self.window_of(block)?;
        let Some(parent) = self.windows.get_mut(&window) else {
            return Err(Error::UnknownWindow(window));
        };
        if !parent.layout.as_ref().is_some_and(|l| l.contains(block)) {
            return Ok(());
        }
        let heir = parent.layout.as_ref().and_then(|l| l.sibling(block));
        parent.layout = parent.layout.take().and_then(|l| l.remove(block));
        if parent.zoomed == Some(block) {
            parent.zoomed = None;
        }
        self.push_layout(window, heir);
        Ok(())
    }

    /// Moves `window` to place `index` among its session's windows,
    /// counted from 0, shifting those between by one.
    pub fn move_window(&mut self, window: WindowId, index: usize) -> Result<(), Error> {
        let session = self
            .windows
            .get(&window)
            .ok_or(Error::UnknownWindow(window))?
            .session;
        let parent = self
            .sessions
            .get_mut(&session)
            .ok_or(Error::UnknownSession(session))?;
        let from = parent
            .windows
            .iter()
            .position(|w| *w == window)
            .ok_or(Error::UnknownWindow(window))?;
        if index >= parent.windows.len() {
            return Err(Error::NoPlace { window, index });
        }
        if index == from {
            return Ok(());
        }
        parent.windows.remove(from);
        parent.windows.insert(index, window);
        self.events.push(Event::WindowMoved { window, index });
        Ok(())
    }

    fn window_of(&self, block: BlockId) -> Result<WindowId, Error> {
        Ok(self
            .blocks
            .get(&block)
            .ok_or(Error::UnknownBlock(block))?
            .window)
    }

    /// The window both blocks are in.
    fn same_window(&self, block: BlockId, other: BlockId) -> Result<WindowId, Error> {
        let window = self.window_of(block)?;
        if self.window_of(other)? != window {
            return Err(Error::OtherWindow { block, other });
        }
        Ok(window)
    }

    fn layout_of(&self, window: WindowId) -> Option<&Layout> {
        self.windows.get(&window).and_then(|w| w.layout.as_ref())
    }

    /// Replaces the window's layout, ending its zoom if `unzoom`, and
    /// announces the result unless nothing changed; focus left without a
    /// place goes to `heir` (see [`Self::push_layout`]).
    fn set_layout(
        &mut self,
        window: WindowId,
        layout: Layout,
        unzoom: bool,
        heir: Option<BlockId>,
    ) {
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        let zoom_ends = unzoom && w.zoomed.is_some();
        if w.layout.as_ref() == Some(&layout) && !zoom_ends {
            return;
        }
        w.layout = Some(layout);
        if zoom_ends {
            w.zoomed = None;
        }
        self.push_layout(window, heir);
    }

    /// Announces the window's layout as it is now, then moves its focus
    /// if it must: to the zoomed block; else, if the focused block left
    /// the layout, to `heir` if placed, or the first block; else nowhere
    /// once none is placed.
    fn push_layout(&mut self, window: WindowId, heir: Option<BlockId>) {
        self.announce_layout(window);
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(layout) = w.layout.as_ref() else {
            return self.set_focus(window, None);
        };
        let kept = w.focused.filter(|b| layout.contains(*b));
        let focus = w
            .zoomed
            .or(kept)
            .or(heir.filter(|b| layout.contains(*b)))
            .or_else(|| layout.blocks().first().copied());
        self.set_focus(window, focus);
    }

    fn announce_layout(&mut self, window: WindowId) {
        if let Some(w) = self.windows.get(&window) {
            self.events.push(Event::LayoutChanged {
                window,
                layout: w.layout.clone(),
                zoomed: w.zoomed,
            });
        }
    }

    /// Focuses `block` in `window`, announcing a change.
    fn set_focus(&mut self, window: WindowId, block: Option<BlockId>) {
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        if w.focused == block {
            return;
        }
        w.focused = block;
        self.events.push(Event::FocusChanged { window, block });
    }

    /// Focuses `block` in its window, so clients send their keys to it. A
    /// zoom on another block ends, so the focused block always shows; a
    /// detached block is refused.
    pub fn focus_block(&mut self, block: BlockId) -> Result<(), Error> {
        let window = self.window_of(block)?;
        if !self.layout_of(window).is_some_and(|l| l.contains(block)) {
            return Err(Error::Detached(block));
        }
        let unzoom = self
            .windows
            .get_mut(&window)
            .filter(|w| w.zoomed.is_some_and(|z| z != block))
            .map(|w| w.zoomed = None)
            .is_some();
        if unzoom {
            self.announce_layout(window);
        }
        self.set_focus(window, Some(block));
        Ok(())
    }

    /// Moves the window's focus from its focused block toward a side or
    /// along the layout's order, as [`Self::focus_block`] does; with no
    /// block that way, nothing changes.
    pub fn move_focus(&mut self, window: WindowId, toward: Toward) -> Result<(), Error> {
        let w = self
            .windows
            .get(&window)
            .ok_or(Error::UnknownWindow(window))?;
        let next = w
            .layout
            .as_ref()
            .zip(w.focused)
            .and_then(|(layout, focused)| layout.neighbour(focused, toward));
        match next {
            Some(block) => self.focus_block(block),
            None => Ok(()),
        }
    }

    /// Resizes the block's PTY. Resizing is input, so it needs the lease.
    pub fn resize_block(
        &mut self,
        client: ClientId,
        block: BlockId,
        size: Size,
    ) -> Result<(), Error> {
        self.authorize_input(client, block)?;
        let target = self
            .blocks
            .get_mut(&block)
            .ok_or(Error::UnknownBlock(block))?;
        if target.size == size {
            return Ok(());
        }
        target.size = size;
        self.events.push(Event::BlockResized { block, size });
        Ok(())
    }

    /// Announces that the block's program status changed. The model holds no
    /// status itself; the host that tracks it calls this.
    pub fn note_status(&mut self, block: BlockId) -> Result<(), Error> {
        self.note(block, Event::BlockStatus { block })
    }

    /// Announces that the block's working directory changed, as
    /// [`Self::note_status`] does its status.
    pub fn note_pwd(&mut self, block: BlockId) -> Result<(), Error> {
        self.note(block, Event::PwdChanged { block })
    }

    /// Announces that the block's title changed, as [`Self::note_status`]
    /// does its status.
    pub fn note_title(&mut self, block: BlockId) -> Result<(), Error> {
        self.note(block, Event::TitleChanged { block })
    }

    /// Announces that the block's program exited by itself, before the host
    /// closes the block.
    pub fn note_exit(&mut self, block: BlockId, status: ExitStatus) -> Result<(), Error> {
        self.note(block, Event::ProgramExited { block, status })
    }

    /// Announces that the block's program rang the bell.
    pub fn note_bell(&mut self, block: BlockId) -> Result<(), Error> {
        self.note(block, Event::Bell { block })
    }

    /// Announces the progress the block's program now shows.
    pub fn note_progress(&mut self, block: BlockId, progress: Progress) -> Result<(), Error> {
        self.note(block, Event::ProgressChanged { block, progress })
    }

    /// Announces a desktop notification the block's program asked for.
    pub fn note_notification(
        &mut self,
        block: BlockId,
        title: String,
        body: String,
    ) -> Result<(), Error> {
        self.note(block, Event::Notification { block, title, body })
    }

    /// Announces that the host read its config again. The model holds no
    /// config; the host that does calls this.
    pub fn note_config_reloaded(&mut self) {
        self.events.push(Event::ConfigReloaded);
    }

    /// Announces that the block's program wrote `selection`.
    pub fn note_clipboard(&mut self, block: BlockId, selection: Selection) -> Result<(), Error> {
        self.note(block, Event::ClipboardWritten { block, selection })
    }

    /// Announces that the block's status record at `id` ended.
    pub fn note_status_removed(&mut self, block: BlockId, id: String) -> Result<(), Error> {
        self.note(block, Event::StatusRemoved { block, id })
    }

    /// Announces that process group `pid`, running `name`, now has the
    /// block's terminal.
    pub fn note_process(&mut self, block: BlockId, pid: u32, name: String) -> Result<(), Error> {
        self.note(block, Event::ProcessChanged { block, pid, name })
    }

    /// Renames the session as [`Mux::rename`] does, but only while its name
    /// is generated ([`Session::generated`]); once somebody chose a name,
    /// it is refused with [`Error::NameChosen`] and the name stays. The
    /// check and the rename happen together, so of two such calls on one
    /// generated session only the first takes effect. Naming it its current
    /// name also ends the mark, though, as with [`Mux::rename`], nothing is
    /// emitted.
    pub fn name_session(
        &mut self,
        session: SessionId,
        name: impl Into<String>,
    ) -> Result<(), Error> {
        let found = self
            .sessions
            .get(&session)
            .ok_or(Error::UnknownSession(session))?;
        if !found.generated {
            return Err(Error::NameChosen(session));
        }
        self.rename(EntityId::Session(session), name)?;
        // A rename to the same name keeps the mark; this one is a choice.
        if let Some(named) = self.sessions.get_mut(&session) {
            named.generated = false;
        }
        Ok(())
    }

    /// Names the entity `name`, replacing its old name. Renaming to the
    /// same name changes nothing and emits nothing; a name over
    /// [`MAX_NAME`] is refused. Renaming a session ends its generated mark
    /// ([`Session::generated`]).
    pub fn rename(&mut self, id: EntityId, name: impl Into<String>) -> Result<(), Error> {
        let name = name.into();
        check_name(&name)?;
        let (slot, event) = match id {
            EntityId::Session(session) => (
                self.sessions
                    .get_mut(&session)
                    .map(|s| &mut s.name)
                    .ok_or(Error::UnknownSession(session))?,
                Event::SessionRenamed {
                    session,
                    name: name.clone(),
                },
            ),
            EntityId::Window(window) => (
                self.windows
                    .get_mut(&window)
                    .map(|w| &mut w.name)
                    .ok_or(Error::UnknownWindow(window))?,
                Event::WindowRenamed {
                    window,
                    name: name.clone(),
                },
            ),
            EntityId::Block(block) => (
                self.blocks
                    .get_mut(&block)
                    .map(|b| &mut b.name)
                    .ok_or(Error::UnknownBlock(block))?,
                Event::BlockRenamed {
                    block,
                    name: name.clone(),
                },
            ),
            EntityId::Client(client) => (
                self.clients
                    .get_mut(&client)
                    .map(|c| &mut c.name)
                    .ok_or(Error::UnknownClient(client))?,
                Event::ClientRenamed {
                    client,
                    name: name.clone(),
                },
            ),
        };
        if *slot == name {
            return Ok(());
        }
        *slot = name;
        // A session renamed has a name somebody chose.
        if let EntityId::Session(session) = id
            && let Some(renamed) = self.sessions.get_mut(&session)
        {
            renamed.generated = false;
        }
        self.events.push(event);
        Ok(())
    }

    /// Clients in id order.
    pub fn clients(&self) -> impl Iterator<Item = &Client> {
        self.clients.values()
    }

    /// Emits `event` about a block the host tracks, if it is live.
    fn note(&mut self, block: BlockId, event: Event) -> Result<(), Error> {
        if !self.blocks.contains_key(&block) {
            return Err(Error::UnknownBlock(block));
        }
        self.events.push(event);
        Ok(())
    }

    /// Closes the session and everything in it.
    pub fn close_session(&mut self, session: SessionId) -> Result<(), Error> {
        let windows = self
            .sessions
            .get(&session)
            .ok_or(Error::UnknownSession(session))?
            .windows
            .clone();
        for window in windows {
            self.remove_window(window);
        }
        self.sessions.remove(&session);
        self.session_order.retain(|id| *id != session);
        self.events.push(Event::SessionClosed { session });
        Ok(())
    }

    /// Closes the window and its blocks.
    pub fn close_window(&mut self, window: WindowId) -> Result<(), Error> {
        let session = self
            .windows
            .get(&window)
            .ok_or(Error::UnknownWindow(window))?
            .session;
        self.remove_window(window);
        let Some(parent) = self.sessions.get_mut(&session) else {
            return Ok(());
        };
        parent.windows.retain(|&w| w != window);
        // The newest window left takes over, as closing a tab does.
        if parent.active == Some(window) {
            parent.active = parent.windows.last().copied();
            self.events.push(Event::ActiveWindowChanged {
                session,
                window: parent.active,
            });
        }
        Ok(())
    }

    /// Closes the block, detaching every client attached to it. Its
    /// sibling takes its split's place, and a zoom on it ends.
    pub fn close_block(&mut self, block: BlockId) -> Result<(), Error> {
        let window = self
            .blocks
            .get(&block)
            .ok_or(Error::UnknownBlock(block))?
            .window;
        let heir = self.layout_of(window).and_then(|l| l.sibling(block));
        self.remove_block(block);
        if let Some(parent) = self.windows.get_mut(&window) {
            parent.blocks.retain(|&b| b != block);
            parent.layout = parent.layout.take().and_then(|l| l.remove(block));
            if parent.zoomed == Some(block) {
                parent.zoomed = None;
            }
        }
        self.push_layout(window, heir);
        Ok(())
    }

    /// Adds a client that the server verified as `principal`.
    pub fn connect(&mut self, principal: Principal, transport: Transport) -> ClientId {
        let id = ClientId::random();
        self.events.push(Event::ClientConnected {
            client: id,
            principal: Some(principal.clone()),
        });
        self.clients.insert(
            id,
            Client {
                id,
                name: String::new(),
                attached: None,
                principal,
                transport,
            },
        );
        id
    }

    /// Records that the client is now reached by `transport`: a new
    /// connection took it over, or its UDP association roamed.
    pub fn reconnect(&mut self, client: ClientId, transport: Transport) -> Result<(), Error> {
        let found = self
            .clients
            .get_mut(&client)
            .ok_or(Error::UnknownClient(client))?;
        found.transport = transport;
        let principal = found.principal.clone();
        self.events.push(Event::ClientConnectionChanged {
            client,
            principal,
            transport,
        });
        Ok(())
    }

    /// Detaches the client, releasing any lease it holds, and forgets it.
    /// Blocks it owns are orphaned: whoever holds their lease loses it.
    pub fn disconnect(&mut self, client: ClientId) -> Result<(), Error> {
        self.detach(client)?;
        self.orphan_blocks_of(client);
        self.end_grants_of(client);
        self.clients.remove(&client);
        self.events.push(Event::ClientDisconnected { client });
        Ok(())
    }

    /// Attaches the client to the block, detaching it from any other block first.
    pub fn attach(&mut self, client: ClientId, block: BlockId) -> Result<(), Error> {
        let current = self
            .clients
            .get(&client)
            .ok_or(Error::UnknownClient(client))?
            .attached;
        if !self.blocks.contains_key(&block) {
            return Err(Error::UnknownBlock(block));
        }
        if current == Some(block) {
            return Ok(());
        }
        self.unattach(client);
        if let Some(c) = self.clients.get_mut(&client) {
            c.attached = Some(block);
        }
        self.events.push(Event::Attached { client, block });
        Ok(())
    }

    /// Detaches the client from its block, if any, releasing the lease it holds.
    pub fn detach(&mut self, client: ClientId) -> Result<(), Error> {
        if !self.clients.contains_key(&client) {
            return Err(Error::UnknownClient(client));
        }
        self.unattach(client);
        Ok(())
    }

    /// Makes `client` the owner of `block`, which has none (C46), with
    /// `input` saying who may type (C77). The server calls it only as the
    /// client's launch creates the block.
    pub fn own(&mut self, client: ClientId, block: BlockId, input: InputMode) -> Result<(), Error> {
        let principal = self
            .clients
            .get(&client)
            .ok_or(Error::UnknownClient(client))?
            .principal
            .clone();
        let target = self
            .blocks
            .get_mut(&block)
            .ok_or(Error::UnknownBlock(block))?;
        if target.owner.is_some() {
            return Err(Error::Owned(block));
        }
        target.owner = Some(Owner {
            principal,
            client: Some(client),
        });
        target.input = input;
        Ok(())
    }

    /// Lets `client` take `block`'s lease until `until` (unix ms), for the
    /// block's owner `owner` (C77). `now` is the server's clock; a grant
    /// ends after it and at most [`MAX_GRANT_MS`] later. A new grant
    /// replaces the client's old one.
    pub fn grant(
        &mut self,
        owner: ClientId,
        block: BlockId,
        client: ClientId,
        until: u64,
        now: u64,
    ) -> Result<(), Error> {
        self.authorize_granter(owner, block)?;
        if !self.clients.contains_key(&client) {
            return Err(Error::UnknownClient(client));
        }
        if until <= now || until - now > MAX_GRANT_MS {
            return Err(Error::BadGrant { block, until });
        }
        if let Some(target) = self.blocks.get_mut(&block) {
            target.grants.insert(client, until);
        }
        self.events.push(Event::GrantChanged {
            block,
            client,
            until: Some(until),
        });
        Ok(())
    }

    /// Ends `client`'s grant on `block`, for its owner `owner`, and takes
    /// the lease back if `client` holds it (C77). Ending no grant changes
    /// nothing.
    pub fn revoke(
        &mut self,
        owner: ClientId,
        block: BlockId,
        client: ClientId,
    ) -> Result<(), Error> {
        self.authorize_granter(owner, block)?;
        self.end_grant(block, client);
        Ok(())
    }

    /// Ends every grant whose time is up at `now` (unix ms), taking back
    /// the lease each one's client holds.
    pub fn expire_grants(&mut self, now: u64) {
        let due: Vec<(BlockId, ClientId)> = self
            .blocks
            .values()
            .flat_map(|b| {
                b.grants
                    .iter()
                    .filter(|&(_, &until)| until <= now)
                    .map(|(&client, _)| (b.id, client))
            })
            .collect();
        for (block, client) in due {
            self.end_grant(block, client);
        }
    }

    /// Succeeds when `owner` owns `block` and may grant its lease.
    fn authorize_granter(&self, owner: ClientId, block: BlockId) -> Result<(), Error> {
        let target = self.blocks.get(&block).ok_or(Error::UnknownBlock(block))?;
        if target.owner.is_none() {
            return Err(Error::Unowned(block));
        }
        self.authorize_owner(owner, block)
    }

    /// Removes `client`'s grant on `block`, if any, and the lease it holds
    /// there.
    fn end_grant(&mut self, block: BlockId, client: ClientId) {
        let Some(target) = self.blocks.get_mut(&block) else {
            return;
        };
        if target.grants.remove(&client).is_none() {
            return;
        }
        self.events.push(Event::GrantChanged {
            block,
            client,
            until: None,
        });
        self.drop_lease_if_held(client, block);
    }

    /// Succeeds when `client` may change `block`: it has no owner, or
    /// `client` owns it (C46).
    pub fn authorize_owner(&self, client: ClientId, block: BlockId) -> Result<(), Error> {
        let target = self.blocks.get(&block).ok_or(Error::UnknownBlock(block))?;
        match target.owner.as_ref().map(|o| o.client) {
            None => Ok(()),
            Some(Some(owner)) if owner == client => Ok(()),
            Some(Some(_)) => Err(Error::NotOwner { client, block }),
            Some(None) => Err(Error::Orphaned(block)),
        }
    }

    /// Succeeds when `client` may close `block`: as [`Self::authorize_owner`],
    /// and also, once the block is orphaned, for a client of its owner's
    /// principal, to clean it up.
    pub fn authorize_close(&self, client: ClientId, block: BlockId) -> Result<(), Error> {
        let refused = match self.authorize_owner(client, block) {
            Err(Error::Orphaned(b)) => Error::Orphaned(b),
            other => return other,
        };
        let caller = self
            .clients
            .get(&client)
            .ok_or(Error::UnknownClient(client))?;
        let theirs = self
            .blocks
            .get(&block)
            .and_then(|b| b.owner.as_ref())
            .is_some_and(|o| o.principal.id() == caller.principal.id());
        if theirs {
            return Ok(());
        }
        Err(refused)
    }

    /// Whether `client` owns any block.
    pub fn owns_any(&self, client: ClientId) -> bool {
        self.blocks
            .values()
            .any(|b| b.owner.as_ref().and_then(|o| o.client) == Some(client))
    }

    /// Gives the client the block's input lease. The client must be attached
    /// to the block, and the lease must be free or already the client's. An
    /// orphaned block's lease is never given.
    pub fn acquire_lease(&mut self, client: ClientId, block: BlockId) -> Result<(), Error> {
        let target = self.attached_block(client, block)?;
        if target.owner.as_ref().is_some_and(Owner::is_orphaned) {
            return Err(Error::Orphaned(block));
        }
        let owner = target.owner.as_ref().and_then(|o| o.client);
        let gated = target.input == InputMode::OwnerOnly
            && owner != Some(client)
            && !target.grants.contains_key(&client);
        if gated {
            return Err(Error::InputGated(block));
        }
        let holder = target.lease;
        match holder {
            Some(h) if h == client => return Ok(()),
            Some(h) => return Err(Error::LeaseHeld { block, holder: h }),
            None => {}
        }
        if let Some(b) = self.blocks.get_mut(&block) {
            b.lease = Some(client);
        }
        self.events.push(Event::LeaseChanged {
            block,
            holder: Some(client),
        });
        Ok(())
    }

    pub fn release_lease(&mut self, client: ClientId, block: BlockId) -> Result<(), Error> {
        self.authorize_input(client, block)?;
        self.drop_lease_if_held(client, block);
        Ok(())
    }

    /// Succeeds when the client may write input to the block.
    pub fn authorize_input(&self, client: ClientId, block: BlockId) -> Result<(), Error> {
        let target = self.attached_block(client, block)?;
        if target.lease == Some(client) {
            return Ok(());
        }
        Err(Error::NoLease { client, block })
    }

    fn attached_block(&self, client: ClientId, block: BlockId) -> Result<&Block, Error> {
        let c = self
            .clients
            .get(&client)
            .ok_or(Error::UnknownClient(client))?;
        let target = self.blocks.get(&block).ok_or(Error::UnknownBlock(block))?;
        if c.attached != Some(block) {
            return Err(Error::NotAttached { client, block });
        }
        Ok(target)
    }

    /// Orphans every block `client` owns, ending the lease on each.
    fn orphan_blocks_of(&mut self, client: ClientId) {
        let owned: Vec<BlockId> = self
            .blocks
            .values()
            .filter(|b| b.owner.as_ref().and_then(|o| o.client) == Some(client))
            .map(|b| b.id)
            .collect();
        for block in owned {
            let Some(target) = self.blocks.get_mut(&block) else {
                continue;
            };
            if let Some(owner) = target.owner.as_mut() {
                owner.client = None;
            }
            if target.lease.take().is_some() {
                self.events.push(Event::LeaseChanged {
                    block,
                    holder: None,
                });
            }
            let granted: Vec<ClientId> = target.grants.keys().copied().collect();
            for client in granted {
                self.end_grant(block, client);
            }
        }
    }

    /// Ends every grant `client` holds: it is gone.
    fn end_grants_of(&mut self, client: ClientId) {
        let granted: Vec<BlockId> = self
            .blocks
            .values()
            .filter(|b| b.grants.contains_key(&client))
            .map(|b| b.id)
            .collect();
        for block in granted {
            self.end_grant(block, client);
        }
    }

    fn unattach(&mut self, client: ClientId) {
        let Some(block) = self
            .clients
            .get_mut(&client)
            .and_then(|c| c.attached.take())
        else {
            return;
        };
        self.drop_lease_if_held(client, block);
        self.events.push(Event::Detached { client, block });
    }

    fn drop_lease_if_held(&mut self, client: ClientId, block: BlockId) {
        let Some(target) = self.blocks.get_mut(&block) else {
            return;
        };
        if target.lease != Some(client) {
            return;
        }
        target.lease = None;
        self.events.push(Event::LeaseChanged {
            block,
            holder: None,
        });
    }

    /// Removes the window and its blocks; the caller unlinks it from its session.
    fn remove_window(&mut self, window: WindowId) {
        let Some(removed) = self.windows.remove(&window) else {
            return;
        };
        for block in removed.blocks {
            self.remove_block(block);
        }
        self.events.push(Event::WindowClosed { window });
    }

    /// Removes the block after detaching its clients; the caller unlinks it from its window.
    fn remove_block(&mut self, block: BlockId) {
        let attached: Vec<ClientId> = self
            .clients
            .values()
            .filter(|c| c.attached == Some(block))
            .map(|c| c.id)
            .collect();
        for client in attached {
            self.unattach(client);
        }
        self.blocks.remove(&block);
        self.events.push(Event::BlockClosed { block });
    }
}

#[cfg(test)]
mod tests;

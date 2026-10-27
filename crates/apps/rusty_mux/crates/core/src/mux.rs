//! The model's state and the operations that change it.

use std::collections::BTreeMap;

use crate::{
    Axis, Block, BlockId, Client, ClientId, EntityId, Error, Event, ExitStatus, Layout, MAX_DEPTH,
    Progress, Session, SessionId, Share, Side, Size, Window, WindowId,
};

/// The longest name any entity may have, in UTF-8 bytes. Names travel in
/// every event and listing, so a bound keeps one name from filling a
/// client's queue or a listing's reply.
pub const MAX_NAME: usize = 256;

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
    sessions: BTreeMap<SessionId, Session>,
    /// Session ids in creation order.
    session_order: Vec<SessionId>,
    windows: BTreeMap<WindowId, Window>,
    blocks: BTreeMap<BlockId, Block>,
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
        let name = name.into();
        check_name(&name)?;
        let id = SessionId::random();
        let session = Session {
            id,
            name,
            windows: Vec::new(),
        };
        self.sessions.insert(id, session);
        self.session_order.push(id);
        self.events.push(Event::SessionCreated { session: id });
        Ok(id)
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
        self.windows.insert(
            id,
            Window {
                id,
                session,
                name,
                blocks: Vec::new(),
                layout: None,
                zoomed: None,
            },
        );
        self.events.push(Event::WindowCreated {
            window: id,
            session,
        });
        Ok(id)
    }

    pub fn create_block(&mut self, window: WindowId, size: Size) -> Result<BlockId, Error> {
        let id = BlockId::random();
        self.add_block(id, window, size)?;
        Ok(id)
    }

    /// Adds a block whose id the caller chose, such as one already given to
    /// the block's program before it started. It splits the window's
    /// largest block, side by side if that block is wider than tall (a cell
    /// being about twice as tall as wide), stacked otherwise; a zoom stays.
    pub fn add_block(&mut self, id: BlockId, window: WindowId, size: Size) -> Result<(), Error> {
        if self.blocks.contains_key(&id) {
            return Err(Error::BlockExists(id));
        }
        let parent = self
            .windows
            .get_mut(&window)
            .ok_or(Error::UnknownWindow(window))?;
        let layout = match parent.layout.take() {
            None => Layout::Block(id),
            Some(layout) => {
                let target = layout.largest();
                let side = match self.blocks.get(&target).map(|b| b.size) {
                    Some(s) if s.cols / 2 < s.rows => Side::Down,
                    _ => Side::Right,
                };
                layout.split(target, id, side)
            }
        };
        parent.layout = Some(layout);
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
            },
        );
        self.events.push(Event::BlockCreated {
            block: id,
            window,
            size,
        });
        self.push_layout(window);
    }

    /// Shows `block` alone over its whole window, until it is unzoomed,
    /// closed, or another block is split off in the window.
    pub fn zoom(&mut self, block: BlockId) -> Result<(), Error> {
        let window = self
            .blocks
            .get(&block)
            .ok_or(Error::UnknownBlock(block))?
            .window;
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
        self.push_layout(window);
        Ok(())
    }

    /// Announces the window's layout as it is now.
    fn push_layout(&mut self, window: WindowId) {
        if let Some(w) = self.windows.get(&window) {
            self.events.push(Event::LayoutChanged {
                window,
                layout: w.layout.clone(),
                zoomed: w.zoomed,
            });
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

    /// Announces that process group `pid`, running `name`, now has the
    /// block's terminal.
    pub fn note_process(&mut self, block: BlockId, pid: u32, name: String) -> Result<(), Error> {
        self.note(block, Event::ProcessChanged { block, pid, name })
    }

    /// Names the entity `name`, replacing its old name. Renaming to the
    /// same name changes nothing and emits nothing; a name over
    /// [`MAX_NAME`] is refused.
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
        if let Some(parent) = self.sessions.get_mut(&session) {
            parent.windows.retain(|&w| w != window);
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
        self.remove_block(block);
        if let Some(parent) = self.windows.get_mut(&window) {
            parent.blocks.retain(|&b| b != block);
            parent.layout = parent.layout.take().and_then(|l| l.remove(block));
            if parent.zoomed == Some(block) {
                parent.zoomed = None;
            }
        }
        self.push_layout(window);
        Ok(())
    }

    pub fn connect(&mut self) -> ClientId {
        let id = ClientId::random();
        self.clients.insert(
            id,
            Client {
                id,
                name: String::new(),
                attached: None,
            },
        );
        self.events.push(Event::ClientConnected { client: id });
        id
    }

    /// Detaches the client, releasing any lease it holds, and forgets it.
    pub fn disconnect(&mut self, client: ClientId) -> Result<(), Error> {
        self.detach(client)?;
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

    /// Gives the client the block's input lease. The client must be attached
    /// to the block, and the lease must be free or already the client's.
    pub fn acquire_lease(&mut self, client: ClientId, block: BlockId) -> Result<(), Error> {
        let holder = self.attached_block(client, block)?.lease;
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

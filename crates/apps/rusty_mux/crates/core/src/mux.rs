//! The model's state and the operations that change it.

use std::collections::BTreeMap;

use crate::{
    Block, BlockId, Client, ClientId, Error, Event, Session, SessionId, Size, Window, WindowId,
};

/// All sessions, windows, blocks and clients, plus the events not yet drained.
///
/// Every operation either succeeds and queues its events, or fails and
/// changes nothing.
#[derive(Debug, Default)]
pub struct Mux {
    sessions: BTreeMap<SessionId, Session>,
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

    pub fn sessions(&self) -> impl Iterator<Item = &Session> {
        self.sessions.values()
    }

    pub fn create_session(&mut self, name: impl Into<String>) -> SessionId {
        let id = SessionId::random();
        let session = Session {
            id,
            name: name.into(),
            windows: Vec::new(),
        };
        self.sessions.insert(id, session);
        self.events.push(Event::SessionCreated { session: id });
        id
    }

    pub fn create_window(
        &mut self,
        session: SessionId,
        name: impl Into<String>,
    ) -> Result<WindowId, Error> {
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
                name: name.into(),
                blocks: Vec::new(),
            },
        );
        self.events.push(Event::WindowCreated {
            window: id,
            session,
        });
        Ok(id)
    }

    pub fn create_block(&mut self, window: WindowId, size: Size) -> Result<BlockId, Error> {
        let parent = self
            .windows
            .get_mut(&window)
            .ok_or(Error::UnknownWindow(window))?;
        let id = BlockId::random();
        parent.blocks.push(id);
        self.blocks.insert(
            id,
            Block {
                id,
                window,
                size,
                lease: None,
            },
        );
        self.events.push(Event::BlockCreated {
            block: id,
            window,
            size,
        });
        Ok(id)
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

    /// Closes the block, detaching every client attached to it.
    pub fn close_block(&mut self, block: BlockId) -> Result<(), Error> {
        let window = self
            .blocks
            .get(&block)
            .ok_or(Error::UnknownBlock(block))?
            .window;
        self.remove_block(block);
        if let Some(parent) = self.windows.get_mut(&window) {
            parent.blocks.retain(|&b| b != block);
        }
        Ok(())
    }

    pub fn connect(&mut self) -> ClientId {
        let id = ClientId::random();
        self.clients.insert(id, Client { id, attached: None });
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

//! The server's sessions as the GUI window shows them, as the server tells
//! it: each session's name, its windows in order with their layouts, the
//! window shown and the clients attached. The top strip draws the current
//! session (handoff D105), the switcher lists them all (gui-design D60),
//! and switching windows or sessions follows it.

use std::collections::{HashMap, HashSet};

use rusty_mux_core::{BlockId, ClientId, Event, Layout, Principal, SessionId, WindowId};
use rusty_mux_proto::{Entry, Message};
use rusty_term::gui::source::{Session, SessionChoice, WindowTab};

use crate::keys::Keys;
use crate::tree::tree;

/// One of the server's sessions.
#[derive(Debug, Clone, PartialEq)]
struct Sess {
    id: SessionId,
    name: String,
    /// Its active window, as the server last said.
    active: Option<WindowId>,
}

/// One of a session's windows.
#[derive(Debug, Clone, PartialEq)]
struct Win {
    id: WindowId,
    session: SessionId,
    name: String,
    layout: Option<Layout>,
    focused: Option<BlockId>,
    /// The block shown alone, if any.
    zoomed: Option<BlockId>,
}

/// A connected client.
#[derive(Debug, Clone, Default, PartialEq)]
struct Client {
    /// Who the server verified it to be.
    principal: Option<Principal>,
    /// Empty until it is named.
    name: String,
    /// The block whose output it receives, if any.
    attached: Option<BlockId>,
}

/// What an event changed for the GUI window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Nothing it shows.
    None,
    /// Only the strip or the switcher.
    Strip,
    /// The shown window's layout, and the strip's thumbnail of it.
    Layout,
    /// The shown window's focus moved to the block.
    Focus(BlockId),
    /// Another window is shown now.
    Shown,
    /// The shown window has no blocks left; the GUI asks for this window
    /// instead, and shows it when the server makes it active.
    Emptied(WindowId),
    /// A client's name, which a block header may show.
    Holders,
    /// Nothing is left to show: the session or its last window closed.
    Gone,
}

/// The server's sessions, and the one the GUI window shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    /// The session shown.
    session: SessionId,
    /// Where the sessions run, as the strip says "on {host}".
    host: String,
    /// Every session, in the order the server listed them.
    sessions: Vec<Sess>,
    /// Every session's windows, each session's in order.
    windows: Vec<Win>,
    /// Each block's window, to tell which session a client is attached to.
    blocks: HashMap<BlockId, WindowId>,
    shown: WindowId,
    /// Every connected client and who the server verified it to be.
    clients: HashMap<ClientId, Client>,
    /// The server tells the session's active window, so the shown window
    /// follows it; without, the GUI ends with its first window.
    follows_active: bool,
}

impl Model {
    /// The sessions in a List's `entries`, showing `block`'s window.
    pub fn from_list(
        entries: &[Message],
        block: BlockId,
        host: String,
        follows_active: bool,
    ) -> Result<Self, String> {
        let shown = entries
            .iter()
            .find_map(|m| match entry(m)? {
                Entry::Block {
                    block: b, window, ..
                } if *b == block => Some(*window),
                _ => None,
            })
            .ok_or_else(|| format!("unknown block {block}"))?;
        let session = entries
            .iter()
            .find_map(|m| match entry(m)? {
                Entry::Window {
                    window, session, ..
                } if *window == shown => Some(*session),
                _ => None,
            })
            .ok_or("the block's window is in no session")?;
        let mut model = Self {
            session,
            host,
            sessions: Vec::new(),
            windows: Vec::new(),
            blocks: HashMap::new(),
            shown,
            clients: HashMap::new(),
            follows_active,
        };
        for m in entries {
            if let Some(e) = entry(m) {
                model.take(e);
            }
        }
        Ok(model)
    }

    /// What a block header calls `client` (gui-design D48): its name, else
    /// who it is, else "Another client".
    pub fn client_name(&self, client: ClientId) -> String {
        let Some(c) = self.clients.get(&client) else {
            return "Another client".into();
        };
        if !c.name.is_empty() {
            return c.name.clone();
        }
        match &c.principal {
            Some(p) if !p.user().is_empty() => p.user().to_owned(),
            Some(p) => p.id().to_owned(),
            None => "Another client".into(),
        }
    }

    /// The blocks laid out in the shown session's windows, for their
    /// headers and the strip's thumbnails.
    pub fn session_blocks(&self) -> Vec<BlockId> {
        self.own_windows()
            .filter_map(|w| w.layout.as_ref())
            .flat_map(Layout::blocks)
            .collect()
    }

    /// The session shown.
    pub fn session_id(&self) -> SessionId {
        self.session
    }

    /// The window shown.
    pub fn shown(&self) -> WindowId {
        self.shown
    }

    /// The shown window's layout and focused block.
    pub fn view(&self) -> (Option<&Layout>, Option<BlockId>) {
        match self.window(self.shown) {
            Some(w) => (w.layout.as_ref(), w.focused),
            None => (None, None),
        }
    }

    /// The block the shown window shows alone, if any.
    pub fn zoomed(&self) -> Option<BlockId> {
        self.window(self.shown).and_then(|w| w.zoomed)
    }

    /// Show `session` instead (gui-design D60, handoff D109): its active
    /// window, else its first with blocks. Whether it is shown now; a
    /// session with no blocks to show, the shown one, or an unknown one
    /// changes nothing.
    pub fn switch(&mut self, session: SessionId) -> bool {
        if session == self.session {
            return false;
        }
        let Some(window) = self.window_to_show(session) else {
            return false;
        };
        self.session = session;
        self.shown = window;
        true
    }

    /// The session as the strip and the switcher show it; sessions keyed
    /// by `sessions`, windows by `windows`, blocks by `blocks`.
    pub fn session(&self, keys: &mut StripKeys<'_>) -> Session {
        let name = self
            .sess(self.session)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        Session {
            name,
            host: self.host.clone(),
            windows: self
                .own_windows()
                .map(|w| WindowTab {
                    key: keys.windows.key(w.id),
                    name: w.name.clone(),
                    tree: w.layout.as_ref().map(|l| tree(l, keys.blocks)),
                    shown: w.id == self.shown,
                })
                .collect(),
            // This client counts, before any of its blocks is listed.
            clients: self.clients_of(self.session).max(1),
            // Reached over a local socket: the server runs here.
            local: true,
            sessions: self
                .sessions
                .iter()
                .map(|s| SessionChoice {
                    key: keys.sessions.key(s.id),
                    name: s.name.clone(),
                    windows: self.windows.iter().filter(|w| w.session == s.id).count(),
                    clients: self.clients_of(s.id),
                    current: s.id == self.session,
                    can_show: self.window_to_show(s.id).is_some(),
                })
                .collect(),
        }
    }

    /// Note a List entry: a new session, window, block or client, or a
    /// name.
    pub fn take(&mut self, entry: &Entry) -> Change {
        match entry {
            Entry::Session {
                session,
                name,
                active,
                ..
            } => {
                let sess = Sess {
                    id: *session,
                    name: name.clone(),
                    active: *active,
                };
                match self.sessions.iter_mut().find(|s| s.id == *session) {
                    Some(s) => *s = sess,
                    None => self.sessions.push(sess),
                }
                Change::Strip
            }
            Entry::Window {
                window,
                session,
                name,
                layout,
                focused,
                zoomed,
            } => {
                let win = Win {
                    id: *window,
                    session: *session,
                    name: name.clone(),
                    layout: layout.clone(),
                    focused: *focused,
                    zoomed: *zoomed,
                };
                match self.windows.iter_mut().find(|w| w.id == *window) {
                    Some(w) => *w = win,
                    None => self.windows.push(win),
                }
                Change::Strip
            }
            Entry::Block { block, window, .. } => match self.blocks.insert(*block, *window) {
                Some(was) if was == *window => Change::None,
                _ => Change::Strip,
            },
            Entry::Client {
                client,
                principal,
                name,
                attached,
                ..
            } => {
                let c = Client {
                    principal: principal.clone(),
                    name: name.clone(),
                    attached: *attached,
                };
                self.clients.insert(*client, c);
                Change::Strip
            }
        }
    }

    /// Apply a server event.
    pub fn apply(&mut self, event: &Event) -> Change {
        match event {
            Event::SessionCreated { session } => {
                if !self.sessions.iter().any(|s| s.id == *session) {
                    self.sessions.push(Sess {
                        id: *session,
                        name: String::new(),
                        active: None,
                    });
                }
                Change::Strip
            }
            Event::SessionRenamed { session, name } => match self.sess_mut(*session) {
                Some(s) => {
                    s.name.clone_from(name);
                    Change::Strip
                }
                None => Change::None,
            },
            Event::SessionClosed { session } if *session == self.session => Change::Gone,
            Event::SessionClosed { session } => {
                self.sessions.retain(|s| s.id != *session);
                self.windows.retain(|w| w.session != *session);
                Change::Strip
            }
            Event::WindowCreated { window, session } => {
                if !self.windows.iter().any(|w| w.id == *window) {
                    self.windows.push(Win {
                        id: *window,
                        session: *session,
                        name: String::new(),
                        layout: None,
                        focused: None,
                        zoomed: None,
                    });
                }
                Change::Strip
            }
            Event::WindowRenamed { window, name } => self.with_window(*window, |w| {
                w.name.clone_from(name);
                Change::Strip
            }),
            Event::WindowMoved { window, index } => self.moved(*window, *index),
            Event::WindowClosed { window } => {
                let Some(at) = self.windows.iter().position(|w| w.id == *window) else {
                    return Change::None;
                };
                self.windows.remove(at);
                // The server makes another window active, which is shown
                // when it says so; an older server can't, so the GUI ends.
                let none_left = !self.own_windows().any(|_| true);
                if *window == self.shown && (!self.follows_active || none_left) {
                    return Change::Gone;
                }
                Change::Strip
            }
            Event::ActiveWindowChanged { session, window } => self.activated(*session, *window),
            Event::LayoutChanged {
                window,
                layout,
                zoomed,
            } => {
                let shown = *window == self.shown;
                let change = self.with_window(*window, |w| {
                    w.layout.clone_from(layout);
                    w.zoomed = *zoomed;
                    if shown { Change::Layout } else { Change::Strip }
                });
                if change != Change::Layout || layout.is_some() || !self.follows_active {
                    return change;
                }
                // The last block of the shown window ended; the server keeps
                // the window. Move to a neighbour that shows blocks, the one
                // before it first; with none, the GUI ends as before.
                match self.neighbour() {
                    Some(next) => Change::Emptied(next),
                    None => Change::Layout,
                }
            }
            Event::FocusChanged { window, block } => {
                let shown = *window == self.shown;
                self.with_window(*window, |w| {
                    w.focused = *block;
                    match block {
                        Some(b) if shown => Change::Focus(*b),
                        _ => Change::None,
                    }
                })
            }
            Event::BlockCreated { block, window, .. } => {
                self.blocks.insert(*block, *window);
                Change::None
            }
            Event::BlockClosed { block } => {
                self.blocks.remove(block);
                Change::None
            }
            Event::ClientConnected { client, principal } => {
                let c = Client {
                    principal: principal.clone(),
                    ..Client::default()
                };
                self.clients.insert(*client, c);
                Change::Strip
            }
            Event::Attached { client, block } => self.attach(*client, Some(*block)),
            Event::Detached { client, .. } => self.attach(*client, None),
            Event::ClientRenamed { client, name } => match self.clients.get_mut(client) {
                Some(c) => {
                    c.name.clone_from(name);
                    Change::Holders
                }
                None => Change::None,
            },
            Event::ClientDisconnected { client } => match self.clients.remove(client) {
                Some(_) => Change::Strip,
                None => Change::None,
            },
            _ => Change::None,
        }
    }

    /// `session`'s active window is `window` now.
    fn activated(&mut self, session: SessionId, window: Option<WindowId>) -> Change {
        let Some(s) = self.sess_mut(session) else {
            return Change::None;
        };
        s.active = window;
        if session != self.session {
            return Change::Strip;
        }
        match window {
            None => Change::Gone,
            Some(w) if w == self.shown => Change::None,
            Some(w) => {
                if self.window(w).is_none() {
                    return Change::None;
                }
                self.shown = w;
                Change::Shown
            }
        }
    }

    /// A client moved `window` to place `index` among its session's.
    fn moved(&mut self, window: WindowId, index: usize) -> Change {
        let Some(from) = self.windows.iter().position(|w| w.id == window) else {
            return Change::None;
        };
        let w = self.windows.remove(from);
        let session = w.session;
        // The place of its session's window now at `index`, else after
        // its session's last.
        let at = self
            .windows
            .iter()
            .enumerate()
            .filter(|(_, x)| x.session == session)
            .map(|(i, _)| i)
            .nth(index)
            .or_else(|| {
                self.windows
                    .iter()
                    .rposition(|x| x.session == session)
                    .map(|i| i + 1)
            })
            .unwrap_or(self.windows.len());
        self.windows.insert(at, w);
        Change::Strip
    }

    /// `client` now receives `block`'s output, or none.
    fn attach(&mut self, client: ClientId, block: Option<BlockId>) -> Change {
        match self.clients.get_mut(&client) {
            Some(c) => {
                c.attached = block;
                Change::Strip
            }
            None => Change::None,
        }
    }

    /// Distinct people attached to `session`'s blocks (handoff D105); a
    /// client the server could not verify counts on its own.
    fn clients_of(&self, session: SessionId) -> usize {
        let mut people = HashSet::new();
        let mut unverified = 0;
        for c in self.clients.values() {
            let in_session = c
                .attached
                .and_then(|b| self.blocks.get(&b))
                .and_then(|w| self.window(*w))
                .is_some_and(|w| w.session == session);
            if !in_session {
                continue;
            }
            match &c.principal {
                Some(p) => {
                    people.insert(p.id());
                }
                None => unverified += 1,
            }
        }
        people.len() + unverified
    }

    /// The window a switch to `session` shows: its active one, else its
    /// first with blocks.
    fn window_to_show(&self, session: SessionId) -> Option<WindowId> {
        let laid_out = |w: &&Win| w.session == session && w.layout.is_some();
        let active = self.sess(session)?.active;
        self.windows
            .iter()
            .filter(laid_out)
            .find(|w| Some(w.id) == active)
            .or_else(|| self.windows.iter().find(laid_out))
            .map(|w| w.id)
    }

    /// The shown session's windows, in order.
    fn own_windows(&self) -> impl Iterator<Item = &Win> {
        self.windows.iter().filter(|w| w.session == self.session)
    }

    /// The nearest window to the shown one that has blocks, earlier ones
    /// first.
    fn neighbour(&self) -> Option<WindowId> {
        let own: Vec<&Win> = self.own_windows().collect();
        let at = own.iter().position(|w| w.id == self.shown)?;
        let (before, after) = own.split_at(at);
        before
            .iter()
            .rev()
            .chain(after.iter().skip(1))
            .find(|w| w.layout.is_some())
            .map(|w| w.id)
    }

    fn sess(&self, session: SessionId) -> Option<&Sess> {
        self.sessions.iter().find(|s| s.id == session)
    }

    fn sess_mut(&mut self, session: SessionId) -> Option<&mut Sess> {
        self.sessions.iter_mut().find(|s| s.id == session)
    }

    fn window(&self, window: WindowId) -> Option<&Win> {
        self.windows.iter().find(|w| w.id == window)
    }

    fn with_window(&mut self, window: WindowId, f: impl FnOnce(&mut Win) -> Change) -> Change {
        match self.windows.iter_mut().find(|w| w.id == window) {
            Some(w) => f(w),
            None => Change::None,
        }
    }
}

/// The keys the window knows the server's ids by.
pub struct StripKeys<'a> {
    pub sessions: &'a mut Keys<SessionId>,
    pub windows: &'a mut Keys<WindowId>,
    pub blocks: &'a mut Keys,
}

/// The entry a List answer carries, if `m` is one.
fn entry(m: &Message) -> Option<&Entry> {
    match m {
        Message::Entry { entry, .. } => Some(entry),
        _ => None,
    }
}

/// This machine's name, for a session reached over a local socket.
pub fn local_host() -> String {
    #[cfg(unix)]
    let name = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok());
    #[cfg(windows)]
    let name = std::env::var("COMPUTERNAME").ok();
    #[cfg(not(any(unix, windows)))]
    let name: Option<String> = None;
    name.map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "localhost".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{Change, Model, StripKeys};
    use crate::keys::Keys;
    use rusty_mux_core::{BlockId, ClientId, Event, Layout, Principal, SessionId, WindowId};
    use rusty_mux_proto::{Entry, Message};
    use rusty_uuid::Uuid;

    fn ids<T>(f: impl Fn(Uuid) -> T) -> [T; 3] {
        [(); 3].map(|_| f(Uuid::new_v4()))
    }

    fn window(window: WindowId, session: SessionId, name: &str, block: BlockId) -> Message {
        Message::Entry {
            req: 1,
            entry: Entry::Window {
                window,
                session,
                name: name.into(),
                layout: Some(Layout::Block(block)),
                zoomed: None,
                focused: Some(block),
            },
        }
    }

    /// A session "work" with windows `w` (blocks `b`), the first shown.
    fn model(s: SessionId, w: &[WindowId; 3], b: &[BlockId; 3]) -> Model {
        let mut list = vec![Message::Entry {
            req: 1,
            entry: Entry::Session {
                session: s,
                name: "work".into(),
                active: Some(w[0]),
                generated: false,
            },
        }];
        for (i, (&w, &b)) in w.iter().zip(b).enumerate() {
            list.push(window(w, s, &format!("w{i}"), b));
            list.push(Message::Entry {
                req: 1,
                entry: Entry::Block {
                    block: b,
                    window: w,
                    size: rusty_mux_core::Size::new(80, 24).expect("a size"),
                    holder: None,
                    title: String::new(),
                    name: String::new(),
                },
            });
        }
        Model::from_list(&list, b[0], "box".into(), true).expect("a model")
    }

    fn strip(m: &Model) -> rusty_term::gui::source::Session {
        m.session(&mut StripKeys {
            sessions: &mut Keys::default(),
            windows: &mut Keys::default(),
            blocks: &mut Keys::default(),
        })
    }

    #[test]
    fn a_list_gives_the_session_its_windows_in_order_and_the_shown_one() {
        let (s, w, b) = (
            SessionId::from_uuid(Uuid::new_v4()),
            ids(WindowId::from_uuid),
            ids(BlockId::from_uuid),
        );
        let m = model(s, &w, &b);
        let got = strip(&m);
        assert_eq!((got.name.as_str(), got.host.as_str()), ("work", "box"));
        let names: Vec<_> = got
            .windows
            .iter()
            .map(|t| (t.name.as_str(), t.shown))
            .collect();
        assert_eq!(names, [("w0", true), ("w1", false), ("w2", false)]);
        assert_eq!(m.view(), (Some(&Layout::Block(b[0])), Some(b[0])));
    }

    #[test]
    fn the_active_window_is_followed_and_unknown_or_same_ignored() {
        let (s, w, b) = (
            SessionId::from_uuid(Uuid::new_v4()),
            ids(WindowId::from_uuid),
            ids(BlockId::from_uuid),
        );
        let mut m = model(s, &w, &b);
        let active = |window| Event::ActiveWindowChanged { session: s, window };
        assert_eq!(m.apply(&active(Some(w[0]))), Change::None);
        assert_eq!(
            m.apply(&active(Some(WindowId::from_uuid(Uuid::new_v4())))),
            Change::None
        );
        assert_eq!(m.apply(&active(Some(w[2]))), Change::Shown);
        assert_eq!((m.shown(), m.view().1), (w[2], Some(b[2])));
        assert_eq!(m.apply(&active(None)), Change::Gone);
    }

    #[test]
    fn closing_the_shown_window_waits_for_the_next_active_one() {
        let (s, w, b) = (
            SessionId::from_uuid(Uuid::new_v4()),
            ids(WindowId::from_uuid),
            ids(BlockId::from_uuid),
        );
        let mut m = model(s, &w, &b);
        assert_eq!(
            m.apply(&Event::WindowClosed { window: w[0] }),
            Change::Strip
        );
        assert_eq!(strip(&m).windows.len(), 2);
        let mut old = model(s, &w, &b);
        old.follows_active = false;
        assert_eq!(
            old.apply(&Event::WindowClosed { window: w[0] }),
            Change::Gone
        );
    }

    #[test]
    fn layout_and_focus_matter_only_for_the_shown_window_but_thumbnails_follow_all() {
        let (s, w, b) = (
            SessionId::from_uuid(Uuid::new_v4()),
            ids(WindowId::from_uuid),
            ids(BlockId::from_uuid),
        );
        let mut m = model(s, &w, &b);
        let changed = |window, layout| Event::LayoutChanged {
            window,
            layout,
            zoomed: None,
        };
        assert_eq!(m.apply(&changed(w[1], None)), Change::Strip);
        assert_eq!(strip(&m).windows[1].tree, None);
        assert_eq!(
            m.apply(&changed(w[0], Some(Layout::Block(b[1])))),
            Change::Layout
        );
        let focus = |window, block| Event::FocusChanged {
            window,
            block: Some(block),
        };
        assert_eq!(m.apply(&focus(w[1], b[1])), Change::None);
        assert_eq!(m.apply(&focus(w[0], b[2])), Change::Focus(b[2]));
    }

    #[test]
    fn the_shown_windows_zoom_is_kept() {
        let (s, w, b) = (
            SessionId::from_uuid(Uuid::new_v4()),
            ids(WindowId::from_uuid),
            ids(BlockId::from_uuid),
        );
        let mut m = model(s, &w, &b);
        assert_eq!(m.zoomed(), None);
        let zoom = |window, zoomed| Event::LayoutChanged {
            window,
            layout: Some(Layout::Block(b[0])),
            zoomed,
        };
        m.apply(&zoom(w[1], Some(b[1])));
        assert_eq!(m.zoomed(), None, "another window's zoom");
        assert_eq!(m.apply(&zoom(w[0], Some(b[0]))), Change::Layout);
        assert_eq!(m.zoomed(), Some(b[0]));
        m.apply(&zoom(w[0], None));
        assert_eq!(m.zoomed(), None);
    }

    #[test]
    fn an_emptied_window_hands_over_to_a_neighbour_or_ends() {
        let (s, w, b) = (
            SessionId::from_uuid(Uuid::new_v4()),
            ids(WindowId::from_uuid),
            ids(BlockId::from_uuid),
        );
        let mut m = model(s, &w, &b);
        let emptied = |window| Event::LayoutChanged {
            window,
            layout: None,
            zoomed: None,
        };
        m.apply(&Event::ActiveWindowChanged {
            session: s,
            window: Some(w[1]),
        });
        assert_eq!(
            m.apply(&emptied(w[1])),
            Change::Emptied(w[0]),
            "the one before first"
        );
        m.apply(&Event::ActiveWindowChanged {
            session: s,
            window: Some(w[0]),
        });
        assert_eq!(
            m.apply(&emptied(w[0])),
            Change::Emptied(w[2]),
            "then the one after"
        );
        m.apply(&Event::ActiveWindowChanged {
            session: s,
            window: Some(w[2]),
        });
        assert_eq!(
            m.apply(&emptied(w[2])),
            Change::Layout,
            "none left: the window ends"
        );
    }

    #[test]
    fn windows_are_created_renamed_and_moved() {
        let (s, w, b) = (
            SessionId::from_uuid(Uuid::new_v4()),
            ids(WindowId::from_uuid),
            ids(BlockId::from_uuid),
        );
        let mut m = model(s, &w, &b);
        let new = WindowId::from_uuid(Uuid::new_v4());
        assert_eq!(
            m.apply(&Event::WindowCreated {
                window: new,
                session: s
            }),
            Change::Strip
        );
        let other = SessionId::from_uuid(Uuid::new_v4());
        let elsewhere = WindowId::from_uuid(Uuid::new_v4());
        assert_eq!(
            m.apply(&Event::WindowCreated {
                window: elsewhere,
                session: other
            }),
            Change::Strip,
            "another session's window: only the switcher's count"
        );
        m.apply(&Event::WindowRenamed {
            window: new,
            name: "logs".into(),
        });
        m.apply(&Event::WindowMoved {
            window: new,
            index: 0,
        });
        let names: Vec<_> = strip(&m).windows.into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["logs", "w0", "w1", "w2"]);
        m.apply(&Event::SessionRenamed {
            session: s,
            name: "play".into(),
        });
        assert_eq!(strip(&m).name, "play");
    }

    #[test]
    fn clients_count_people_once_and_never_zero() {
        let (s, w, b) = (
            SessionId::from_uuid(Uuid::new_v4()),
            ids(WindowId::from_uuid),
            ids(BlockId::from_uuid),
        );
        let mut m = model(s, &w, &b);
        assert_eq!(strip(&m).clients, 1, "this client, before any is listed");
        let me = Principal::new("uid:1000", "me").expect("a principal");
        let you = Principal::new("uid:1001", "you").expect("a principal");
        let [c1, c2, c3] = ids(ClientId::from_uuid);
        for (c, p, block) in [(c1, &me, b[0]), (c2, &me, b[1]), (c3, &you, b[2])] {
            m.apply(&Event::ClientConnected {
                client: c,
                principal: Some(p.clone()),
            });
            m.apply(&Event::Attached { client: c, block });
        }
        assert_eq!(
            strip(&m).clients,
            2,
            "two connections of one person count once"
        );
        m.apply(&Event::ClientDisconnected { client: c3 });
        assert_eq!(strip(&m).clients, 1);
        m.apply(&Event::Detached {
            client: c1,
            block: b[0],
        });
        m.apply(&Event::Detached {
            client: c2,
            block: b[1],
        });
        let control = ClientId::from_uuid(Uuid::new_v4());
        m.apply(&Event::ClientConnected {
            client: control,
            principal: Some(you.clone()),
        });
        assert_eq!(
            strip(&m).clients,
            1,
            "a client attached to no block is not counted, this one is"
        );
    }

    /// A second session "play" with one window `w` holding block `b`,
    /// added to `m` as a List would.
    fn second(m: &mut Model, s: SessionId, w: WindowId, b: BlockId, active: bool) {
        m.take(&Entry::Session {
            session: s,
            name: "play".into(),
            active: active.then_some(w),
            generated: false,
        });
        if let Message::Entry { entry, .. } = window(w, s, "p0", b) {
            m.take(&entry);
        }
        m.take(&Entry::Block {
            block: b,
            window: w,
            size: rusty_mux_core::Size::new(80, 24).expect("a size"),
            holder: None,
            title: String::new(),
            name: String::new(),
        });
    }

    #[test]
    fn every_session_is_listed_and_only_the_shown_ones_windows_are_tabs() {
        let (s, w, b) = (
            SessionId::from_uuid(Uuid::new_v4()),
            ids(WindowId::from_uuid),
            ids(BlockId::from_uuid),
        );
        let mut m = model(s, &w, &b);
        let [s2] = [SessionId::from_uuid(Uuid::new_v4())];
        let [w2, b2] = [Uuid::new_v4(), Uuid::new_v4()];
        let (w2, b2) = (WindowId::from_uuid(w2), BlockId::from_uuid(b2));
        second(&mut m, s2, w2, b2, true);
        let got = strip(&m);
        assert_eq!(got.windows.len(), 3, "the other session's window is no tab");
        let rows: Vec<_> = got
            .sessions
            .iter()
            .map(|c| (c.name.as_str(), c.windows, c.current, c.can_show))
            .collect();
        assert_eq!(rows, [("work", 3, true, true), ("play", 1, false, true)]);
        let blocks = m.session_blocks();
        assert_eq!(blocks, b.to_vec(), "the shown session's blocks only");
    }

    #[test]
    fn switching_shows_the_active_window_else_the_first_with_blocks() {
        let (s, w, b) = (
            SessionId::from_uuid(Uuid::new_v4()),
            ids(WindowId::from_uuid),
            ids(BlockId::from_uuid),
        );
        let mut m = model(s, &w, &b);
        let s2 = SessionId::from_uuid(Uuid::new_v4());
        let (w2, b2) = (
            WindowId::from_uuid(Uuid::new_v4()),
            BlockId::from_uuid(Uuid::new_v4()),
        );
        second(&mut m, s2, w2, b2, false);
        assert!(!m.switch(s), "the shown session changes nothing");
        assert!(m.switch(s2));
        assert_eq!((m.session_id(), m.shown()), (s2, w2));
        assert_eq!(m.view(), (Some(&Layout::Block(b2)), Some(b2)));
        let names: Vec<_> = strip(&m).windows.into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["p0"]);
        // Back: the session's active window, not its first.
        m.apply(&Event::ActiveWindowChanged {
            session: s,
            window: Some(w[2]),
        });
        assert!(m.switch(s));
        assert_eq!(m.shown(), w[2]);
    }

    #[test]
    fn a_session_without_blocks_cannot_be_picked() {
        let (s, w, b) = (
            SessionId::from_uuid(Uuid::new_v4()),
            ids(WindowId::from_uuid),
            ids(BlockId::from_uuid),
        );
        let mut m = model(s, &w, &b);
        let s2 = SessionId::from_uuid(Uuid::new_v4());
        m.apply(&Event::SessionCreated { session: s2 });
        assert!(!strip(&m).sessions[1].can_show);
        assert!(!m.switch(s2));
        assert!(!m.switch(SessionId::from_uuid(Uuid::new_v4())), "unknown");
        assert_eq!(m.session_id(), s);
    }

    #[test]
    fn other_sessions_change_only_the_switcher() {
        let (s, w, b) = (
            SessionId::from_uuid(Uuid::new_v4()),
            ids(WindowId::from_uuid),
            ids(BlockId::from_uuid),
        );
        let mut m = model(s, &w, &b);
        let s2 = SessionId::from_uuid(Uuid::new_v4());
        let (w2, b2) = (
            WindowId::from_uuid(Uuid::new_v4()),
            BlockId::from_uuid(Uuid::new_v4()),
        );
        second(&mut m, s2, w2, b2, true);
        let active = Event::ActiveWindowChanged {
            session: s2,
            window: None,
        };
        assert_eq!(m.apply(&active), Change::Strip, "not Gone");
        assert_eq!(
            m.apply(&Event::SessionRenamed {
                session: s2,
                name: "night".into()
            }),
            Change::Strip
        );
        assert_eq!(strip(&m).sessions[1].name, "night");
        assert_eq!(
            m.apply(&Event::SessionClosed { session: s2 }),
            Change::Strip
        );
        assert_eq!(strip(&m).sessions.len(), 1);
        assert_eq!(m.shown(), w[0]);
    }
}

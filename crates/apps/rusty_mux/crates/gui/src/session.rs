//! The session the GUI window shows, as the server tells it: its name, its
//! windows in order with their layouts, the window shown and the clients
//! attached. The top strip draws it (handoff D105); switching windows
//! follows it.

use std::collections::HashMap;

use rusty_mux_core::{BlockId, ClientId, Event, Layout, Principal, SessionId, WindowId};
use rusty_mux_proto::{Entry, Message};
use rusty_term::gui::source::{Session, WindowTab};

use crate::keys::Keys;
use crate::tree::tree;

/// One of the session's windows.
#[derive(Debug, Clone, PartialEq)]
struct Win {
    id: WindowId,
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
}

/// What an event changed for the GUI window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Nothing it shows.
    None,
    /// Only the strip.
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

/// The GUI window's session.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    session: SessionId,
    name: String,
    /// Where the session runs, as the strip says "on {host}".
    host: String,
    windows: Vec<Win>,
    shown: WindowId,
    /// Every connected client and who the server verified it to be.
    clients: HashMap<ClientId, Client>,
    /// The server tells the session's active window, so the shown window
    /// follows it; without, the GUI ends with its first window.
    follows_active: bool,
}

impl Model {
    /// The session of `block`'s window, from a List's `entries`.
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
            name: String::new(),
            host,
            windows: Vec::new(),
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

    /// The shown window's blocks.
    pub fn shown_blocks(&self) -> Vec<BlockId> {
        self.view().0.map(Layout::blocks).unwrap_or_default()
    }

    /// The session's id.
    pub fn session_id(&self) -> SessionId {
        self.session
    }

    /// The window shown.
    pub fn shown(&self) -> WindowId {
        self.shown
    }

    /// The shown window's layout and focused block.
    pub fn view(&self) -> (Option<&Layout>, Option<BlockId>) {
        match self.windows.iter().find(|w| w.id == self.shown) {
            Some(w) => (w.layout.as_ref(), w.focused),
            None => (None, None),
        }
    }

    /// The block the shown window shows alone, if any.
    pub fn zoomed(&self) -> Option<BlockId> {
        self.windows
            .iter()
            .find(|w| w.id == self.shown)
            .and_then(|w| w.zoomed)
    }

    /// The session the server said, for the strip; windows keyed by
    /// `windows`, blocks by `blocks`.
    pub fn session(&self, windows: &mut Keys<WindowId>, blocks: &mut Keys) -> Session {
        // Clients without a verified principal are told apart by id.
        let mut people: Vec<Option<&Principal>> = self
            .clients
            .values()
            .map(|c| c.principal.as_ref())
            .collect();
        let unverified = people.iter().filter(|p| p.is_none()).count();
        people.retain(Option::is_some);
        people.sort_by(|a, b| a.map(Principal::id).cmp(&b.map(Principal::id)));
        people.dedup();
        Session {
            name: self.name.clone(),
            host: self.host.clone(),
            windows: self
                .windows
                .iter()
                .map(|w| WindowTab {
                    key: windows.key(w.id),
                    name: w.name.clone(),
                    tree: w.layout.as_ref().map(|l| tree(l, blocks)),
                    shown: w.id == self.shown,
                })
                .collect(),
            clients: (people.len() + unverified).max(1),
            // Reached over a local socket: the server runs here.
            local: true,
        }
    }

    /// Note a List entry: a new window or client, or a name.
    pub fn take(&mut self, entry: &Entry) -> Change {
        match entry {
            Entry::Session { session, name, .. } if *session == self.session => {
                self.name.clone_from(name);
                Change::Strip
            }
            Entry::Window {
                window,
                session,
                name,
                layout,
                focused,
                zoomed,
                ..
            } if *session == self.session => {
                let win = Win {
                    id: *window,
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
            Entry::Client {
                client,
                principal,
                name,
                ..
            } => {
                let c = Client {
                    principal: principal.clone(),
                    name: name.clone(),
                };
                self.clients.insert(*client, c);
                Change::Strip
            }
            _ => Change::None,
        }
    }

    /// Apply a server event.
    pub fn apply(&mut self, event: &Event) -> Change {
        match event {
            Event::SessionRenamed { session, name } if *session == self.session => {
                self.name.clone_from(name);
                Change::Strip
            }
            Event::SessionClosed { session } if *session == self.session => Change::Gone,
            Event::WindowCreated { window, session } if *session == self.session => {
                if !self.windows.iter().any(|w| w.id == *window) {
                    self.windows.push(Win {
                        id: *window,
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
            Event::WindowMoved { window, index } => {
                let Some(from) = self.windows.iter().position(|w| w.id == *window) else {
                    return Change::None;
                };
                let w = self.windows.remove(from);
                let to = (*index).min(self.windows.len());
                self.windows.insert(to, w);
                Change::Strip
            }
            Event::WindowClosed { window } => {
                let Some(at) = self.windows.iter().position(|w| w.id == *window) else {
                    return Change::None;
                };
                self.windows.remove(at);
                // The server makes another window active, which is shown
                // when it says so; an older server can't, so the GUI ends.
                if *window == self.shown && (!self.follows_active || self.windows.is_empty()) {
                    return Change::Gone;
                }
                Change::Strip
            }
            Event::ActiveWindowChanged { session, window } if *session == self.session => {
                match window {
                    None => Change::Gone,
                    Some(w) if *w == self.shown => Change::None,
                    Some(w) => {
                        if !self.windows.iter().any(|x| x.id == *w) {
                            return Change::None;
                        }
                        self.shown = *w;
                        Change::Shown
                    }
                }
            }
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
            Event::ClientConnected { client, principal } => {
                let c = Client {
                    principal: principal.clone(),
                    name: String::new(),
                };
                self.clients.insert(*client, c);
                Change::Strip
            }
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

    /// The nearest window to the shown one that has blocks, earlier ones
    /// first.
    fn neighbour(&self) -> Option<WindowId> {
        let at = self.windows.iter().position(|w| w.id == self.shown)?;
        let (before, after) = self.windows.split_at(at);
        before
            .iter()
            .rev()
            .chain(after.iter().skip(1))
            .find(|w| w.layout.is_some())
            .map(|w| w.id)
    }

    fn with_window(&mut self, window: WindowId, f: impl FnOnce(&mut Win) -> Change) -> Change {
        match self.windows.iter_mut().find(|w| w.id == window) {
            Some(w) => f(w),
            None => Change::None,
        }
    }
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
    use super::{Change, Model};
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
        m.session(&mut Keys::default(), &mut Keys::default())
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
            Change::None
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
        for (c, p) in [(c1, &me), (c2, &me), (c3, &you)] {
            m.apply(&Event::ClientConnected {
                client: c,
                principal: Some(p.clone()),
            });
        }
        assert_eq!(
            strip(&m).clients,
            2,
            "two connections of one person count once"
        );
        m.apply(&Event::ClientDisconnected { client: c3 });
        assert_eq!(strip(&m).clients, 1);
    }
}

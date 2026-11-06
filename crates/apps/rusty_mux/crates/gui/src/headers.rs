//! What each shown block's header says (gui-design D48), as the server
//! tells it (handoff D105): its folder, its status report and its process
//! chain by `BlockCall`, read again when an event says one changed, and who
//! holds its input lease from List entries and `LeaseChanged`.
//!
//! No I/O: [`Headers`] says which calls to send, takes their answers and
//! the events, and gives the window each block's [`BlockInfo`].

use std::collections::HashMap;

use rusty_mux_core::{BlockId, ClientId, Event};
use rusty_mux_proto::Entry;
use rusty_mux_status::{Kind, Report, State as Said, parse};
use rusty_term::gui::source::{BlockInfo, State, Status, Wait};

use crate::keys::Keys;

/// A block call the headers need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    /// `terminal.pwd`: the folder.
    Pwd,
    /// `terminal.status`: OSC 7501 report bodies, the root first.
    Status,
    /// `terminal.process`: the foreground process first, the block's own
    /// program last.
    Process,
}

impl Call {
    /// The method's name on the wire.
    pub fn method(self) -> &'static str {
        match self {
            Call::Pwd => "terminal.pwd",
            Call::Status => "terminal.status",
            Call::Process => "terminal.process",
        }
    }
}

/// A process: its id and name.
type Proc = (u32, String);

/// What is known of one block.
#[derive(Debug, Clone, Default, PartialEq)]
struct Known {
    cwd: Option<String>,
    /// The block's own program, its shell.
    shell: Option<Proc>,
    foreground: Option<Proc>,
    status: Option<Status>,
    holder: Option<ClientId>,
}

/// The shown blocks' headers, and the calls asked for them.
#[derive(Debug, Default)]
pub struct Headers {
    known: HashMap<BlockId, Known>,
    pending: HashMap<u64, (BlockId, Call)>,
}

impl Headers {
    /// Follow exactly the `shown` blocks; returns the calls that learn the
    /// ones new to it.
    pub fn track(&mut self, shown: &[BlockId]) -> Vec<(BlockId, Call)> {
        self.known.retain(|b, _| shown.contains(b));
        let mut calls = Vec::new();
        for &block in shown {
            if self.known.contains_key(&block) {
                continue;
            }
            self.known.insert(block, Known::default());
            calls.extend([Call::Pwd, Call::Status, Call::Process].map(|c| (block, c)));
        }
        calls
    }

    /// Call `call` on `block` went out as request `req`.
    pub fn asked(&mut self, req: u64, block: BlockId, call: Call) {
        self.pending.insert(req, (block, call));
    }

    /// Note a List entry: a followed block's lease holder. Whether it
    /// changed a header.
    pub fn take(&mut self, entry: &Entry) -> bool {
        match entry {
            Entry::Block { block, holder, .. } => self.with(*block, |k| {
                let changed = k.holder != *holder;
                k.holder = *holder;
                changed
            }),
            _ => false,
        }
    }

    /// Apply a server event: what it changed, or the call that reads what
    /// it says changed.
    pub fn apply(&mut self, event: &Event) -> Effect {
        let ask = |block: &BlockId, call| match self.known.contains_key(block) {
            true => Effect::Ask(*block, call),
            false => Effect::None,
        };
        match event {
            Event::PwdChanged { block } => ask(block, Call::Pwd),
            Event::BlockStatus { block } | Event::StatusRemoved { block, .. } => {
                ask(block, Call::Status)
            }
            Event::ProcessChanged { block, pid, name } => {
                let changed = self.with(*block, |k| {
                    let fg = Some((*pid, name.clone()));
                    let changed = k.foreground != fg;
                    k.foreground = fg;
                    changed
                });
                Effect::changed(changed)
            }
            Event::LeaseChanged { block, holder } => Effect::changed(self.with(*block, |k| {
                let changed = k.holder != *holder;
                k.holder = *holder;
                changed
            })),
            _ => Effect::None,
        }
    }

    /// Request `req`'s answer: its text, or `None` when it failed. Whether
    /// it changed a header.
    pub fn answer(&mut self, req: u64, text: Option<&str>) -> bool {
        let Some((block, call)) = self.pending.remove(&req) else {
            return false;
        };
        let text = text.unwrap_or_default();
        self.with(block, |k| {
            let before = k.clone();
            match call {
                Call::Pwd => k.cwd = Some(text.trim_end().to_owned()).filter(|c| !c.is_empty()),
                Call::Status => k.status = status(text),
                Call::Process => {
                    let chain: Vec<Proc> = text.lines().filter_map(process).collect();
                    k.foreground = chain.first().cloned();
                    k.shell = chain.last().cloned();
                }
            }
            *k != before
        })
    }

    /// Whether `req` is a call asked here.
    pub fn owns(&self, req: u64) -> bool {
        self.pending.contains_key(&req)
    }

    /// Each followed block's header facts, by pane key. A lease held by
    /// one of `mine` (this window's own connections) shows no holder;
    /// `name` names any other.
    pub fn infos(
        &self,
        keys: &mut Keys,
        mine: &[ClientId],
        name: impl Fn(ClientId) -> String,
    ) -> Vec<(u64, BlockInfo)> {
        let mut out: Vec<(u64, BlockInfo)> = self
            .known
            .iter()
            .map(|(&block, k)| {
                let foreground = match (&k.foreground, &k.shell) {
                    (Some(fg), Some(shell)) if fg.0 == shell.0 => None,
                    (fg, _) => fg.as_ref().map(|p| p.1.clone()).filter(|n| !n.is_empty()),
                };
                let info = BlockInfo {
                    cwd: k.cwd.clone(),
                    shell: k
                        .shell
                        .as_ref()
                        .map(|p| p.1.clone())
                        .filter(|n| !n.is_empty()),
                    foreground,
                    status: k.status.clone(),
                    holder: k.holder.filter(|h| !mine.contains(h)).map(&name),
                };
                (keys.key(block), info)
            })
            .collect();
        out.sort_by_key(|(key, _)| *key);
        out
    }

    fn with(&mut self, block: BlockId, f: impl FnOnce(&mut Known) -> bool) -> bool {
        self.known.get_mut(&block).is_some_and(f)
    }
}

/// What an event means for the headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    None,
    /// A header changed.
    Changed,
    /// Read this again.
    Ask(BlockId, Call),
}

impl Effect {
    fn changed(changed: bool) -> Self {
        if changed {
            Effect::Changed
        } else {
            Effect::None
        }
    }
}

/// The top record of a `terminal.status` answer.
fn status(text: &str) -> Option<Status> {
    let Some(Report::Set(_, record)) = text.lines().next().and_then(parse) else {
        return None;
    };
    let state = match record.state {
        Said::Idle => State::Idle,
        Said::Working => State::Working,
        Said::Blocked => State::Blocked(record.kind.map(|k| match k {
            Kind::Permission => Wait::Permission,
            Kind::Question => Wait::Question,
            Kind::Auth => Wait::SignIn,
        })),
        Said::Done => State::Done,
        Said::Error => State::Error,
    };
    Some(Status {
        state,
        msg: record.msg.filter(|m| !m.is_empty()),
    })
}

/// One `terminal.process` line: `PID<TAB>NAME<TAB>COMMAND LINE`.
fn process(line: &str) -> Option<Proc> {
    let mut fields = line.split('\t');
    let pid = fields.next()?.parse().ok()?;
    Some((pid, fields.next().unwrap_or_default().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::{Call, Effect, Headers};
    use crate::keys::Keys;
    use rusty_mux_core::{BlockId, ClientId, Event, Size, WindowId};
    use rusty_mux_proto::Entry;
    use rusty_term::gui::source::{BlockInfo, State, Wait};
    use rusty_uuid::Uuid;

    /// A followed block, this window's connection and another client.
    struct Ids {
        block: BlockId,
        mine: ClientId,
        other: ClientId,
    }

    /// Headers following a block, its three calls asked as requests 1-3.
    fn followed() -> (Headers, Ids) {
        let ids = Ids {
            block: BlockId::from_uuid(Uuid::new_v4()),
            mine: ClientId::from_uuid(Uuid::new_v4()),
            other: ClientId::from_uuid(Uuid::new_v4()),
        };
        let mut h = Headers::default();
        let calls = h.track(&[ids.block]);
        assert_eq!(calls.len(), 3);
        for (req, (b, c)) in (1..).zip(calls) {
            h.asked(req, b, c);
        }
        (h, ids)
    }

    fn info(h: &Headers, ids: &Ids) -> BlockInfo {
        let mut keys = Keys::default();
        let other = ids.other;
        let mut infos = h.infos(&mut keys, &[ids.mine], |c| match c == other {
            true => "laptop".into(),
            false => "?".into(),
        });
        assert_eq!(infos.len(), 1);
        infos.remove(0).1
    }

    #[test]
    fn answers_fill_the_header() {
        let (mut h, ids) = followed();
        assert!(h.answer(1, Some("/home/nano/src\n")));
        assert!(h.answer(2, Some("state=working:msg=RWRpdGluZw==\n")));
        assert!(h.answer(3, Some("42\tclaude\tclaude\n7\tbash\t-bash\n")));
        let i = info(&h, &ids);
        assert_eq!(i.cwd.as_deref(), Some("/home/nano/src"));
        assert_eq!(i.shell.as_deref(), Some("bash"));
        assert_eq!(i.foreground.as_deref(), Some("claude"));
        let status = i.status.expect("a status");
        assert_eq!(status.state, State::Working);
        assert_eq!(status.msg.as_deref(), Some("Editing"));
        assert!(!h.answer(3, Some("x")), "each answer is taken once");
    }

    #[test]
    fn the_shell_in_the_foreground_is_no_foreground_program() {
        let (mut h, ids) = followed();
        h.answer(3, Some("7\tbash\t-bash\n"));
        assert_eq!(info(&h, &ids).foreground, None);
        let e = Event::ProcessChanged {
            block: ids.block,
            pid: 50,
            name: "vim".into(),
        };
        assert_eq!(h.apply(&e), Effect::Changed);
        assert_eq!(info(&h, &ids).foreground.as_deref(), Some("vim"));
    }

    #[test]
    fn a_blocked_report_says_why_and_a_failed_call_knows_nothing() {
        let (mut h, ids) = followed();
        h.answer(2, Some("state=blocked:kind=permission\n"));
        let s = info(&h, &ids).status.expect("a status");
        assert_eq!(s.state, State::Blocked(Some(Wait::Permission)));
        assert!(!h.answer(1, None), "nothing was known");
        assert_eq!(info(&h, &ids).cwd, None);
    }

    #[test]
    fn events_ask_again_for_followed_blocks_only() {
        let (mut h, ids) = followed();
        let pwd = Event::PwdChanged { block: ids.block };
        assert_eq!(h.apply(&pwd), Effect::Ask(ids.block, Call::Pwd));
        let other = Event::BlockStatus {
            block: BlockId::from_uuid(Uuid::new_v4()),
        };
        assert_eq!(h.apply(&other), Effect::None);
    }

    #[test]
    fn another_holder_is_named_and_mine_is_not() {
        let (mut h, ids) = followed();
        let lease = |holder| Event::LeaseChanged {
            block: ids.block,
            holder,
        };
        assert_eq!(h.apply(&lease(Some(ids.other))), Effect::Changed);
        assert_eq!(info(&h, &ids).holder.as_deref(), Some("laptop"));
        h.apply(&lease(Some(ids.mine)));
        assert_eq!(info(&h, &ids).holder, None, "this window holds it");
        let entry = Entry::Block {
            block: ids.block,
            window: WindowId::from_uuid(Uuid::new_v4()),
            size: Size::new(80, 24).expect("a size"),
            holder: None,
            title: String::new(),
            name: String::new(),
        };
        assert!(h.take(&entry), "a List entry says nobody holds it");
    }

    #[test]
    fn tracking_forgets_blocks_no_longer_shown() {
        let (mut h, ids) = followed();
        assert!(h.track(&[ids.block]).is_empty(), "already known");
        let calls = h.track(&[BlockId::from_uuid(Uuid::new_v4())]);
        assert_eq!(calls.len(), 3);
        let mut keys = Keys::default();
        assert_eq!(h.infos(&mut keys, &[], |_| String::new()).len(), 1);
    }
}

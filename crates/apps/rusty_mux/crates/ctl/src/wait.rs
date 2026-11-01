//! `wait` and `run`: block until a block's program exits, until every
//! block in a window or session has ended, or until an event whose fields
//! match, on one connection.
//!
//! The event stream starts before anything is looked up or created, so an
//! exit that happens meanwhile is not missed: events that arrive while a
//! listing or a `create-block` is answered are kept and checked first.
//!
//! A window or session wait takes its blocks from the listing, which
//! already reflects every kept event, and then follows the stream: a block
//! or window created in it joins, a block whose program exits or that
//! closes leaves. A block that ended after the stream began but before the
//! listing is left out of it, yet still counts: closing a block announces
//! its window's layout at the very next number, which names the window
//! that `block-closed` does not. Blocks of a whole window closed in that
//! moment send no layout and are not counted. Blocks never move between
//! windows today; a move (C41) must make the block join or leave here too.

use std::collections::BTreeSet;

use rusty_mux_core::{Event, ExitStatus};
use rusty_mux_proto::schema::{Entity, Value};
use rusty_mux_proto::{Entry, Features, Message};
use rusty_uuid::Uuid;

use crate::cli::{Run, Scope, Until};
use crate::{Exit, Failure, Reply, Session, render, target, watch};

/// What a wait looks for, with its targets resolved to ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Goal {
    /// The block's program exits, or the block closes.
    Exit(Uuid),
    /// Every block in a window or session ends.
    Ended(Group),
    /// An event of `kind` whose fields have these values.
    Event {
        kind: &'static str,
        fields: Vec<(&'static str, String)>,
    },
}

/// What one event means to a wait.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Seen {
    /// Nothing for this wait.
    Passed,
    /// A step on the way, printed.
    Shown,
    /// The wait ends, with its exit code or the reason it failed; printed.
    Ends(Result<Exit, String>),
}

impl Goal {
    /// What `event` means to the wait, updating what it follows.
    pub(crate) fn see(&mut self, event: &Event) -> Seen {
        match self {
            Self::Exit(block) => match event {
                Event::ProgramExited { block: b, status } if b.uuid() == *block => {
                    Seen::Ends(Ok(Exit::of_program(*status)))
                }
                Event::BlockClosed { block: b } if b.uuid() == *block => {
                    Seen::Ends(Err(closed_early(*block)))
                }
                _ => Seen::Passed,
            },
            Self::Ended(group) => group.see(event),
            Self::Event { kind, fields } => {
                let (seen, values) = render::event_fields(event);
                let matches = seen == *kind
                    && fields
                        .iter()
                        .all(|(name, want)| values.iter().any(|(n, v)| n == name && v == want));
                match matches {
                    true => Seen::Ends(Ok(Exit::Ok)),
                    false => Seen::Passed,
                }
            }
        }
    }

    /// How the wait ends before any event: only a window or session with
    /// no block left in it ends at once.
    fn settled(&self) -> Option<Result<Exit, String>> {
        match self {
            Self::Ended(group) if group.blocks.is_empty() => Some(group.outcome.clone()),
            _ => None,
        }
    }
}

fn closed_early(block: Uuid) -> String {
    format!("block {block} closed before its program exited")
}

/// A window or session followed until no block is left in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Group {
    scope: Scope,
    id: Uuid,
    /// The windows whose new blocks join: the window itself, or the
    /// session's windows.
    windows: BTreeSet<Uuid>,
    /// The blocks still running.
    blocks: BTreeSet<Uuid>,
    /// How the wait ends: `Ok` until a block fails, then the last failure,
    /// as `set -o pipefail` reports a pipeline.
    outcome: Result<Exit, String>,
}

impl Group {
    /// The window or session `id` with its windows and blocks in `entries`.
    pub(crate) fn new(scope: Scope, id: Uuid, entries: &[Entry]) -> Self {
        let windows: BTreeSet<Uuid> = match scope {
            Scope::Window => BTreeSet::from([id]),
            Scope::Session => entries
                .iter()
                .filter_map(|entry| match entry {
                    Entry::Window {
                        window, session, ..
                    } if session.uuid() == id => Some(window.uuid()),
                    _ => None,
                })
                .collect(),
        };
        let blocks = entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Block { block, window, .. } if windows.contains(&window.uuid()) => {
                    Some(block.uuid())
                }
                _ => None,
            })
            .collect();
        Self {
            scope,
            id,
            windows,
            blocks,
            outcome: Ok(Exit::Ok),
        }
    }

    /// Takes back the blocks of its windows that closed while the listing
    /// was awaited, so their exits in `kept` count.
    pub(crate) fn adopt(&mut self, kept: &Kept) {
        for pair in kept.windows(2) {
            let [
                (seq, Event::BlockClosed { block }),
                (next, Event::LayoutChanged { window, .. }),
            ] = pair
            else {
                continue;
            };
            if seq.checked_add(1) == Some(*next) && self.windows.contains(&window.uuid()) {
                self.blocks.insert(block.uuid());
            }
        }
    }

    fn see(&mut self, event: &Event) -> Seen {
        match event {
            Event::WindowCreated { window, session }
                if self.scope == Scope::Session && session.uuid() == self.id =>
            {
                self.windows.insert(window.uuid());
                Seen::Passed
            }
            Event::BlockCreated { block, window, .. } if self.windows.contains(&window.uuid()) => {
                self.blocks.insert(block.uuid());
                Seen::Passed
            }
            Event::ProgramExited { block, status } if self.blocks.remove(&block.uuid()) => {
                let exit = Exit::of_program(*status);
                if exit != Exit::Ok {
                    self.outcome = Ok(exit);
                }
                self.left()
            }
            Event::BlockClosed { block } if self.blocks.remove(&block.uuid()) => {
                self.outcome = Err(closed_early(block.uuid()));
                self.left()
            }
            // Closing a window or session closes its blocks first, so the
            // wait has ended by the time either is announced.
            Event::WindowClosed { window } => {
                self.windows.remove(&window.uuid());
                Seen::Passed
            }
            _ => Seen::Passed,
        }
    }

    /// A block left: the wait ends with the last one.
    fn left(&self) -> Seen {
        match self.blocks.is_empty() {
            true => Seen::Ends(self.outcome.clone()),
            false => Seen::Shown,
        }
    }
}

/// The exit code a shell would report for a program's status.
impl Exit {
    /// `Ok` for code 0, the code itself up to 255, and 128 plus the signal
    /// that ended it; a code outside 0..=255, as Windows can return, is
    /// `Failed`, its value still printed.
    pub fn of_program(status: ExitStatus) -> Self {
        let code = match status {
            ExitStatus::Code(code) => u8::try_from(code).ok(),
            ExitStatus::Signal(signal) => u8::try_from(signal)
                .ok()
                .and_then(|s| s.checked_add(128))
                .filter(|_| signal > 0),
        };
        match code.map(std::num::NonZeroU8::new) {
            Some(None) => Self::Ok,
            Some(Some(code)) => Self::Program(code),
            None => Self::Failed,
        }
    }
}

/// An event kept while a reply was awaited, with its number.
pub(crate) type Kept = Vec<(u64, Event)>;

impl<S: std::io::Read + std::io::Write> Session<'_, S> {
    /// `wait`: until the block's program exits or a matching event comes.
    pub(crate) fn wait(&mut self, until: &Until) -> Result<Exit, Failure> {
        let needs = match until {
            Until::Exit { .. } => Features::PROGRAM_EVENTS,
            // The layout names the window a block closed in; see `adopt`.
            Until::Ended { .. } => Features::PROGRAM_EVENTS.union(Features::LAYOUT),
            Until::Event { kind, .. } => watch::needs(kind),
        };
        self.open_stream(needs.union(Features::INSPECT), "wait")?;
        let mut kept = Kept::new();
        let entries = self.list_keeping(&mut kept)?;
        let goal = match until {
            Until::Exit { block } => Goal::Exit(self.waited_block(block, &entries, &kept)?),
            Until::Ended { scope, target } => {
                let entity = match scope {
                    Scope::Window => Entity::Window,
                    Scope::Session => Entity::Session,
                };
                let id = target::resolve(&entries, entity, target, self.here)?;
                let mut group = Group::new(*scope, id, &entries);
                group.adopt(&kept);
                Goal::Ended(group)
            }
            Until::Event { kind, fields } => {
                let fields = fields
                    .iter()
                    .map(|(name, text)| Ok((*name, self.field(name, text, &entries)?)))
                    .collect::<Result<_, Failure>>()?;
                Goal::Event { kind, fields }
            }
        };
        self.until(goal, kept)
    }

    /// `run`: creates the block, prints its id, and with `--wait` waits for
    /// its program.
    pub(crate) fn run(&mut self, run: &Run) -> Result<Exit, Failure> {
        let call = &run.create;
        let given = call.args.iter().map(|(name, _)| *name);
        let mut needs = call.method.needs_for(given);
        if call.has_target() {
            needs = needs.union(Features::INSPECT);
        }
        if run.wait.is_some() {
            needs = needs.union(Features::PROGRAM_EVENTS);
        }
        self.open_stream(needs, "run")?;
        let mut kept = Kept::new();
        let entries = match call.has_target() {
            true => self.list_keeping(&mut kept)?,
            false => Vec::new(),
        };
        let args = self.resolve(call, &entries)?;
        let message = call
            .method
            .request(REQ, &args)
            .map_err(|e| Failure::Failed(e.to_string()))?;
        self.conn.send(&message)?;
        let Reply::Done {
            created: Some(block),
            ..
        } = self.reply_keeping(REQ, REQ, call.method.name, Some(&mut kept))?
        else {
            return Err(Failure::Failed(
                "create-block answered without an id".into(),
            ));
        };
        self.print(
            call.method.name,
            Reply::Done {
                created: Some(block),
                entries: Vec::new(),
            },
        )?;
        match run.wait {
            Some(_) => self.until(Goal::Exit(block), kept),
            None => Ok(Exit::Ok),
        }
    }

    /// Starts a fresh numbered event stream, failing unless the server
    /// has `needs` too. Asks for [`Features::LABELS`] besides, so that a
    /// listing includes clients for targets to name.
    fn open_stream(&mut self, needs: Features, what: &str) -> Result<(), Failure> {
        let needs = needs.union(Features::EVENT_SEQ);
        let (features, _) = self.hello(needs.union(Features::LABELS), None)?;
        if features.contains(needs) {
            return Ok(());
        }
        Err(Failure::Failed(format!(
            "the server does not support everything {what} needs here"
        )))
    }

    fn list_keeping(&mut self, kept: &mut Kept) -> Result<Vec<rusty_mux_proto::Entry>, Failure> {
        self.conn.send(&Message::List { req: LIST })?;
        match self.reply_keeping(LIST, LIST, "list", Some(kept))? {
            Reply::Done { entries, .. } => Ok(entries),
            Reply::Text(_) => Err(Failure::Failed("list answered with text".into())),
        }
    }

    /// The block `spec` names. A full id of a block that closed after the
    /// stream started still counts, so its exit is reported.
    fn waited_block(
        &self,
        spec: &str,
        entries: &[rusty_mux_proto::Entry],
        kept: &Kept,
    ) -> Result<Uuid, Failure> {
        if let Ok(id) = spec.parse::<Uuid>()
            && kept
                .iter()
                .any(|(_, e)| Goal::Exit(id).see(e) != Seen::Passed)
        {
            return Ok(id);
        }
        Ok(target::resolve(entries, Entity::Block, spec, self.here)?)
    }

    /// A filter's value: a session, window, block or client target as its id.
    fn field(
        &self,
        name: &str,
        text: &str,
        entries: &[rusty_mux_proto::Entry],
    ) -> Result<String, Failure> {
        let entity = match name {
            "session" => Entity::Session,
            "window" => Entity::Window,
            "block" => Entity::Block,
            "client" => Entity::Client,
            _ => return Ok(text.to_owned()),
        };
        if let Ok(Value::Id(id)) = rusty_mux_proto::schema::Ty::Id(entity).parse(text) {
            return Ok(id.to_string());
        }
        Ok(target::resolve(entries, entity, text, self.here)?.to_string())
    }

    /// Checks the kept events, then each new one, until one ends the wait;
    /// prints that event and every step before it.
    fn until(&mut self, mut goal: Goal, kept: Kept) -> Result<Exit, Failure> {
        for (seq, event) in kept {
            if let Some(end) = self.end(&mut goal, seq, &event)? {
                return end;
            }
        }
        if let Some(end) = goal.settled() {
            return end.map_err(Failure::Failed);
        }
        loop {
            if let Message::Event {
                seq: Some(seq),
                event,
            } = self.conn.recv()?
                && let Some(end) = self.end(&mut goal, seq, &event)?
            {
                return end;
            }
        }
    }

    /// Prints `event` if it matters to the wait; how the wait ends, if it does.
    fn end(
        &mut self,
        goal: &mut Goal,
        seq: u64,
        event: &Event,
    ) -> Result<Option<Result<Exit, Failure>>, Failure> {
        let end = match goal.see(event) {
            Seen::Passed => return Ok(None),
            Seen::Shown => None,
            Seen::Ends(end) => Some(end),
        };
        self.line(&render::event_line(seq, event, self.json))?;
        Ok(end.map(|end| end.map_err(Failure::Failed)))
    }
}

/// Request numbers: the listing, then the request `run` sends.
const LIST: u64 = 1;
const REQ: u64 = 2;

#[cfg(test)]
mod tests {
    use std::num::NonZeroU8;

    use rusty_mux_core::{BlockId, Progress, SessionId, Size, WindowId};

    use super::*;

    fn id(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    #[test]
    fn a_program_status_is_the_exit_a_shell_reports() {
        let program = |n| Exit::Program(NonZeroU8::new(n).expect("non-zero"));
        for (status, exit) in [
            (ExitStatus::Code(0), Exit::Ok),
            (ExitStatus::Code(3), program(3)),
            (ExitStatus::Code(255), program(255)),
            (ExitStatus::Code(256), Exit::Failed),
            (ExitStatus::Code(-1073741819), Exit::Failed),
            (ExitStatus::Signal(9), program(137)),
            (ExitStatus::Signal(127), program(255)),
            (ExitStatus::Signal(128), Exit::Failed),
        ] {
            assert_eq!(Exit::of_program(status), exit, "{status:?}");
        }
        assert_eq!(program(137).code(), 137);
        assert_eq!(Exit::TimedOut.code(), 124);
    }

    #[test]
    fn an_exit_wait_ends_on_its_block_only() {
        let goal = Goal::Exit(id(1));
        let block = BlockId::from_uuid(id(1));
        let other = BlockId::from_uuid(id(2));
        let exited = |block| Event::ProgramExited {
            block,
            status: ExitStatus::Code(0),
        };
        let mut goal = goal;
        assert_eq!(goal.see(&exited(other)), Seen::Passed);
        assert_eq!(goal.see(&Event::Bell { block }), Seen::Passed);
        assert_eq!(goal.see(&exited(block)), Seen::Ends(Ok(Exit::Ok)));
        assert!(matches!(
            goal.see(&Event::BlockClosed { block }),
            Seen::Ends(Err(why)) if why.contains("closed before")
        ));
    }

    #[test]
    fn an_event_wait_matches_the_kind_and_every_field() {
        let block = BlockId::from_uuid(id(1));
        let progress = |p| Event::ProgressChanged {
            block,
            progress: Progress::Normal(p),
        };
        let mut goal = Goal::Event {
            kind: "progress-changed",
            fields: vec![("block", id(1).to_string()), ("percent", "100".into())],
        };
        assert_eq!(goal.see(&progress(40)), Seen::Passed);
        assert_eq!(goal.see(&Event::Bell { block }), Seen::Passed);
        assert_eq!(goal.see(&progress(100)), Seen::Ends(Ok(Exit::Ok)));
        // A field the event leaves out, such as a cleared bar's percent,
        // never matches.
        let cleared = Event::ProgressChanged {
            block,
            progress: Progress::Cleared,
        };
        assert_eq!(goal.see(&cleared), Seen::Passed);
        let mut any = Goal::Event {
            kind: "bell",
            fields: Vec::new(),
        };
        assert_eq!(any.see(&Event::Bell { block }), Seen::Ends(Ok(Exit::Ok)));
    }

    /// A listing of session 1 with windows 2 and 3, blocks 4 and 5 in
    /// window 2, and block 6 in window 7 of session 8.
    fn listing() -> Vec<Entry> {
        let window = |w: u8, s: u8| Entry::Window {
            window: WindowId::from_uuid(id(w)),
            session: SessionId::from_uuid(id(s)),
            name: String::new(),
            layout: None,
            zoomed: None,
        };
        let block = |b: u8, w: u8| Entry::Block {
            block: BlockId::from_uuid(id(b)),
            window: WindowId::from_uuid(id(w)),
            size: size(),
            holder: None,
            title: String::new(),
            name: String::new(),
        };
        vec![
            Entry::Session {
                session: SessionId::from_uuid(id(1)),
                name: "s".into(),
            },
            window(2, 1),
            window(3, 1),
            window(7, 8),
            block(4, 2),
            block(5, 2),
            block(6, 7),
        ]
    }

    fn size() -> Size {
        Size::new(80, 24).expect("a size")
    }

    fn exited(b: u8, code: i32) -> Event {
        Event::ProgramExited {
            block: BlockId::from_uuid(id(b)),
            status: ExitStatus::Code(code),
        }
    }

    fn closed(b: u8) -> Event {
        Event::BlockClosed {
            block: BlockId::from_uuid(id(b)),
        }
    }

    fn created(b: u8, w: u8) -> Event {
        Event::BlockCreated {
            block: BlockId::from_uuid(id(b)),
            window: WindowId::from_uuid(id(w)),
            size: size(),
        }
    }

    fn program(code: u8) -> Exit {
        Exit::Program(NonZeroU8::new(code).expect("non-zero"))
    }

    #[test]
    fn a_window_wait_ends_with_its_last_block_and_its_last_failure() {
        let mut goal = Goal::Ended(Group::new(Scope::Window, id(2), &listing()));
        assert_eq!(goal.settled(), None);
        // Another window's block, and a block's close after its exit,
        // change nothing.
        assert_eq!(goal.see(&exited(6, 1)), Seen::Passed);
        assert_eq!(goal.see(&exited(4, 3)), Seen::Shown);
        assert_eq!(goal.see(&closed(4)), Seen::Passed);
        // A block made in the window joins.
        assert_eq!(goal.see(&created(9, 2)), Seen::Passed);
        assert_eq!(goal.see(&created(10, 3)), Seen::Passed);
        assert_eq!(goal.see(&exited(5, 5)), Seen::Shown);
        // The later failure wins; a success after it does not clear it.
        assert_eq!(goal.see(&exited(9, 0)), Seen::Ends(Ok(program(5))));
    }

    #[test]
    fn a_window_wait_succeeds_only_if_every_program_does() {
        let mut goal = Goal::Ended(Group::new(Scope::Window, id(2), &listing()));
        assert_eq!(goal.see(&exited(4, 0)), Seen::Shown);
        assert_eq!(goal.see(&exited(5, 0)), Seen::Ends(Ok(Exit::Ok)));
        let mut goal = Goal::Ended(Group::new(Scope::Window, id(2), &listing()));
        assert_eq!(goal.see(&exited(4, 0)), Seen::Shown);
        assert!(matches!(
            goal.see(&closed(5)),
            Seen::Ends(Err(why)) if why.contains("closed before its program exited")
        ));
    }

    #[test]
    fn a_block_that_ended_before_the_listing_still_counts() {
        // Block 4 exited with 3 and closed before the listing, which shows
        // only block 5 in window 2.
        let entries: Vec<Entry> = listing()
            .into_iter()
            .filter(|e| !matches!(e, Entry::Block { block, .. } if block.uuid() == id(4)))
            .collect();
        let layout = |w: u8| Event::LayoutChanged {
            window: WindowId::from_uuid(id(w)),
            layout: None,
            zoomed: None,
        };
        let kept: Kept = vec![(10, exited(4, 3)), (11, closed(4)), (12, layout(2))];
        let mut group = Group::new(Scope::Window, id(2), &entries);
        group.adopt(&kept);
        let mut goal = Goal::Ended(group);
        let seen: Vec<Seen> = kept.iter().map(|(_, e)| goal.see(e)).collect();
        assert_eq!(seen, [Seen::Shown, Seen::Passed, Seen::Passed]);
        assert_eq!(goal.see(&exited(5, 0)), Seen::Ends(Ok(program(3))));

        // Another window's layout, or one that is not the very next
        // event, adopts nothing.
        for kept in [
            vec![(11, closed(4)), (12, layout(7))],
            vec![(11, closed(4)), (13, layout(2))],
        ] {
            let mut group = Group::new(Scope::Window, id(2), &entries);
            group.adopt(&kept);
            assert_eq!(Goal::Ended(group).see(&exited(4, 3)), Seen::Passed);
        }
    }

    #[test]
    fn a_session_wait_follows_new_windows_and_ends_at_once_when_empty() {
        let mut goal = Goal::Ended(Group::new(Scope::Session, id(1), &listing()));
        assert_eq!(goal.see(&created(9, 3)), Seen::Passed);
        let window = Event::WindowCreated {
            window: WindowId::from_uuid(id(11)),
            session: SessionId::from_uuid(id(1)),
        };
        assert_eq!(goal.see(&window), Seen::Passed);
        assert_eq!(goal.see(&created(12, 11)), Seen::Passed);
        // A window of another session never joins.
        let foreign = Event::WindowCreated {
            window: WindowId::from_uuid(id(13)),
            session: SessionId::from_uuid(id(8)),
        };
        assert_eq!(goal.see(&foreign), Seen::Passed);
        assert_eq!(goal.see(&created(14, 13)), Seen::Passed);
        for block in [4, 5, 9] {
            assert_eq!(goal.see(&closed(block)), Seen::Shown, "{block}");
        }
        assert!(matches!(goal.see(&exited(12, 0)), Seen::Ends(Err(_))));

        // A window with no blocks ends before any event, successfully.
        let empty = Goal::Ended(Group::new(Scope::Window, id(3), &listing()));
        assert_eq!(empty.settled(), Some(Ok(Exit::Ok)));
    }
}

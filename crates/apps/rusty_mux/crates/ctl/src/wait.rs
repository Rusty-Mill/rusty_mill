//! `wait` and `run`: block until a block's program exits, or until an
//! event whose fields match, on one connection.
//!
//! The event stream starts before anything is looked up or created, so an
//! exit that happens meanwhile is not missed: events that arrive while a
//! listing or a `create-block` is answered are kept and checked first.

use rusty_mux_core::{Event, ExitStatus};
use rusty_mux_proto::schema::{Entity, Value};
use rusty_mux_proto::{Features, Message};
use rusty_uuid::Uuid;

use crate::cli::{Run, Until};
use crate::{Exit, Failure, Reply, Session, render, target, watch};

/// What a wait looks for, with its targets resolved to ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Goal {
    /// The block's program exits, or the block closes.
    Exit(Uuid),
    /// An event of `kind` whose fields have these values.
    Event {
        kind: &'static str,
        fields: Vec<(&'static str, String)>,
    },
}

impl Goal {
    /// How the wait ends if `event` arrives: with an exit code, or with
    /// the reason it failed; `None` if it goes on.
    pub(crate) fn ends(&self, event: &Event) -> Option<Result<Exit, String>> {
        match self {
            Self::Exit(block) => match event {
                Event::ProgramExited { block: b, status } if b.uuid() == *block => {
                    Some(Ok(Exit::of_program(*status)))
                }
                Event::BlockClosed { block: b } if b.uuid() == *block => Some(Err(format!(
                    "block {block} closed before its program exited"
                ))),
                _ => None,
            },
            Self::Event { kind, fields } => {
                let (seen, values) = render::event_fields(event);
                let matches = seen == *kind
                    && fields
                        .iter()
                        .all(|(name, want)| values.iter().any(|(n, v)| n == name && v == want));
                matches.then_some(Ok(Exit::Ok))
            }
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
            Until::Event { kind, .. } => watch::needs(kind),
        };
        self.open_stream(needs.union(Features::INSPECT), "wait")?;
        let mut kept = Kept::new();
        let entries = self.list_keeping(&mut kept)?;
        let goal = match until {
            Until::Exit { block } => Goal::Exit(self.waited_block(block, &entries, &kept)?),
            Until::Event { kind, fields } => {
                let fields = fields
                    .iter()
                    .map(|(name, text)| Ok((*name, self.field(name, text, &entries)?)))
                    .collect::<Result<_, Failure>>()?;
                Goal::Event { kind, fields }
            }
        };
        self.until(&goal, kept)
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
            Some(_) => self.until(&Goal::Exit(block), kept),
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
            && kept.iter().any(|(_, e)| Goal::Exit(id).ends(e).is_some())
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
    /// prints that event.
    fn until(&mut self, goal: &Goal, kept: Kept) -> Result<Exit, Failure> {
        for (seq, event) in kept {
            if let Some(end) = self.end(goal, seq, &event)? {
                return end;
            }
        }
        loop {
            if let Message::Event {
                seq: Some(seq),
                event,
            } = self.conn.recv()?
                && let Some(end) = self.end(goal, seq, &event)?
            {
                return end;
            }
        }
    }

    fn end(
        &mut self,
        goal: &Goal,
        seq: u64,
        event: &Event,
    ) -> Result<Option<Result<Exit, Failure>>, Failure> {
        let Some(end) = goal.ends(event) else {
            return Ok(None);
        };
        self.line(&render::event_line(seq, event, self.json))?;
        Ok(Some(end.map_err(Failure::Failed)))
    }
}

/// Request numbers: the listing, then the request `run` sends.
const LIST: u64 = 1;
const REQ: u64 = 2;

#[cfg(test)]
mod tests {
    use std::num::NonZeroU8;

    use rusty_mux_core::{BlockId, Progress};

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
        assert_eq!(goal.ends(&exited(other)), None);
        assert_eq!(goal.ends(&Event::Bell { block }), None);
        assert_eq!(goal.ends(&exited(block)), Some(Ok(Exit::Ok)));
        assert!(matches!(
            goal.ends(&Event::BlockClosed { block }),
            Some(Err(why)) if why.contains("closed before")
        ));
    }

    #[test]
    fn an_event_wait_matches_the_kind_and_every_field() {
        let block = BlockId::from_uuid(id(1));
        let progress = |p| Event::ProgressChanged {
            block,
            progress: Progress::Normal(p),
        };
        let goal = Goal::Event {
            kind: "progress-changed",
            fields: vec![("block", id(1).to_string()), ("percent", "100".into())],
        };
        assert_eq!(goal.ends(&progress(40)), None);
        assert_eq!(goal.ends(&Event::Bell { block }), None);
        assert_eq!(goal.ends(&progress(100)), Some(Ok(Exit::Ok)));
        // A field the event leaves out, such as a cleared bar's percent,
        // never matches.
        let cleared = Event::ProgressChanged {
            block,
            progress: Progress::Cleared,
        };
        assert_eq!(goal.ends(&cleared), None);
        let any = Goal::Event {
            kind: "bell",
            fields: Vec::new(),
        };
        assert_eq!(any.ends(&Event::Bell { block }), Some(Ok(Exit::Ok)));
    }
}

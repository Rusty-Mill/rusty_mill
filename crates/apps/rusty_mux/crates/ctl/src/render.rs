//! Output lines: plain text for people, one JSON object per line with
//! `--json`.

use rusty_json::{Map, Value};
use rusty_mux_core::{Event, ExitStatus, Progress, Size};
use rusty_mux_proto::{Cursor, Entry};
use rusty_uuid::Uuid;

/// An event or entry field's value.
enum Field {
    Id(Uuid),
    Num(i64),
    /// A fixed word, such as a progress state.
    Word(&'static str),
    Text(String),
    /// No value, such as a lease nobody holds.
    Absent,
}

/// `EPOCH:SEQ`, the form `--after` accepts.
pub(crate) fn cursor_text(cursor: Cursor) -> String {
    format!("{}:{}", cursor.epoch, cursor.seq)
}

pub(crate) fn cursor_line(cursor: Cursor, json: bool) -> String {
    let text = cursor_text(cursor);
    if !json {
        return format!("cursor {text}");
    }
    object([("cursor", Value::from(text))])
}

/// A succeeded request: its new id in text, if it made one.
pub(crate) fn done_line(method: &str, created: Option<Uuid>, json: bool) -> Option<String> {
    if !json {
        return created.map(|id| id.to_string());
    }
    let mut fields = vec![("method", Value::from(method)), ("ok", Value::Bool(true))];
    if let Some(id) = created {
        fields.push(("created", Value::from(id.to_string())));
    }
    Some(object(fields))
}

/// A refused request, for stdout with `--json` (text goes to stderr).
pub(crate) fn failed_line(method: &str, reason: &str) -> String {
    object([
        ("method", Value::from(method)),
        ("ok", Value::Bool(false)),
        ("error", Value::from(reason)),
    ])
}

/// A captured screen: the text itself, or a JSON object holding it.
pub(crate) fn text_line(method: &str, text: String, json: bool) -> String {
    if !json {
        return text;
    }
    object([
        ("method", Value::from(method)),
        ("ok", Value::Bool(true)),
        ("text", Value::from(text)),
    ])
}

pub(crate) fn event_line(seq: u64, event: &Event, json: bool) -> String {
    let (kind, fields) = describe(event);
    if json {
        return object_with(
            [("seq", Value::from(seq)), ("kind", Value::from(kind))],
            fields,
        );
    }
    with_fields(format!("{seq} {kind}"), fields)
}

/// An event's kind and its fields' values, as a rule's placeholders take
/// them (see [`rusty_mux_script::EVENTS`]); a field with no value, such as
/// a free lease's holder, is left out.
pub(crate) fn event_fields(event: &Event) -> (&'static str, Vec<(String, String)>) {
    let (kind, fields) = describe(event);
    let values = fields
        .into_iter()
        .filter_map(|(name, field)| {
            let value = match field {
                Field::Id(id) => id.to_string(),
                Field::Num(n) => n.to_string(),
                Field::Word(word) => word.to_owned(),
                Field::Text(text) => text,
                Field::Absent => return None,
            };
            Some((name.to_owned(), value))
        })
        .collect();
    (kind, values)
}

/// One entity of a listing: `KIND ID field=value…` in text.
pub(crate) fn entry_line(entry: &Entry, json: bool) -> String {
    let (kind, id, fields) = describe_entry(entry);
    if json {
        let head = [
            ("kind", Value::from(kind)),
            (kind, Value::from(id.to_string())),
        ];
        return object_with(head, fields);
    }
    with_fields(format!("{kind} {id}"), fields)
}

fn with_fields(mut line: String, fields: Vec<(&str, Field)>) -> String {
    for (name, field) in fields {
        line.push_str(&format!(" {name}={}", field.text()));
    }
    line
}

fn object_with<'a>(head: [(&'a str, Value); 2], fields: Vec<(&'a str, Field)>) -> String {
    object(
        head.into_iter()
            .chain(fields.into_iter().map(|(k, f)| (k, f.json()))),
    )
}

impl Field {
    fn json(self) -> Value {
        match self {
            Self::Id(id) => Value::from(id.to_string()),
            Self::Num(n) => Value::from(n),
            Self::Word(w) => Value::from(w),
            Self::Text(t) => Value::from(t),
            Self::Absent => Value::Null,
        }
    }

    fn text(self) -> String {
        match self {
            Self::Id(id) => id.to_string(),
            Self::Num(n) => n.to_string(),
            Self::Word(w) => w.to_owned(),
            // Quoted and escaped: a name can't add a line or reach the terminal.
            Self::Text(t) => format!("\"{}\"", t.escape_debug()),
            Self::Absent => "-".into(),
        }
    }
}

fn object<'a>(fields: impl IntoIterator<Item = (&'a str, Value)>) -> String {
    let mut map = Map::new();
    for (key, value) in fields {
        map.insert(key.into(), value);
    }
    Value::Object(map).to_json_string()
}

fn size_fields(size: Size) -> [(&'static str, Field); 2] {
    [
        ("cols", Field::Num(size.cols().into())),
        ("rows", Field::Num(size.rows().into())),
    ]
}

fn describe_entry(entry: &Entry) -> (&'static str, Uuid, Vec<(&'static str, Field)>) {
    use Field::{Id, Text};
    match entry {
        Entry::Session { session, name } => (
            "session",
            session.uuid(),
            vec![("name", Text(name.clone()))],
        ),
        Entry::Window {
            window,
            session,
            name,
        } => (
            "window",
            window.uuid(),
            vec![
                ("session", Id(session.uuid())),
                ("name", Text(name.clone())),
            ],
        ),
        Entry::Block {
            block,
            window,
            size,
            holder,
            title,
        } => {
            let mut fields = vec![("window", Id(window.uuid()))];
            fields.extend(size_fields(*size));
            fields.push(("holder", holder.map_or(Field::Absent, |c| Id(c.uuid()))));
            fields.push(("title", Text(title.clone())));
            ("block", block.uuid(), fields)
        }
    }
}

fn describe(event: &Event) -> (&'static str, Vec<(&'static str, Field)>) {
    use Event as E;
    use Field::{Id, Text};
    match *event {
        E::SessionCreated { session } => ("session-created", vec![("session", Id(session.uuid()))]),
        E::SessionClosed { session } => ("session-closed", vec![("session", Id(session.uuid()))]),
        E::WindowCreated { window, session } => (
            "window-created",
            vec![
                ("window", Id(window.uuid())),
                ("session", Id(session.uuid())),
            ],
        ),
        E::WindowClosed { window } => ("window-closed", vec![("window", Id(window.uuid()))]),
        E::BlockCreated {
            block,
            window,
            size,
        } => {
            let mut fields = vec![("block", Id(block.uuid())), ("window", Id(window.uuid()))];
            fields.extend(size_fields(size));
            ("block-created", fields)
        }
        E::BlockResized { block, size } => {
            let mut fields = vec![("block", Id(block.uuid()))];
            fields.extend(size_fields(size));
            ("block-resized", fields)
        }
        E::BlockClosed { block } => ("block-closed", vec![("block", Id(block.uuid()))]),
        E::ClientConnected { client } => ("client-connected", vec![("client", Id(client.uuid()))]),
        E::ClientDisconnected { client } => {
            ("client-disconnected", vec![("client", Id(client.uuid()))])
        }
        E::Attached { client, block } => (
            "attached",
            vec![("client", Id(client.uuid())), ("block", Id(block.uuid()))],
        ),
        E::Detached { client, block } => (
            "detached",
            vec![("client", Id(client.uuid())), ("block", Id(block.uuid()))],
        ),
        E::LeaseChanged { block, holder } => (
            "lease-changed",
            vec![
                ("block", Id(block.uuid())),
                ("holder", holder.map_or(Field::Absent, |c| Id(c.uuid()))),
            ],
        ),
        E::BlockStatus { block } => ("block-status", vec![("block", Id(block.uuid()))]),
        E::PwdChanged { block } => ("pwd-changed", vec![("block", Id(block.uuid()))]),
        E::TitleChanged { block } => ("title-changed", vec![("block", Id(block.uuid()))]),
        E::ProgramExited { block, status } => {
            let (code, signal) = match status {
                ExitStatus::Code(code) => (Field::Num(code.into()), Field::Absent),
                ExitStatus::Signal(signal) => (Field::Absent, Field::Num(signal.into())),
            };
            (
                "program-exited",
                vec![
                    ("block", Id(block.uuid())),
                    ("code", code),
                    ("signal", signal),
                ],
            )
        }
        E::Bell { block } => ("bell", vec![("block", Id(block.uuid()))]),
        E::ProgressChanged { block, progress } => {
            let (state, percent) = match progress {
                Progress::Cleared => ("cleared", None),
                Progress::Normal(p) => ("normal", Some(p)),
                Progress::Error(p) => ("error", Some(p)),
                Progress::Indeterminate => ("indeterminate", None),
                Progress::Paused(p) => ("paused", Some(p)),
            };
            (
                "progress-changed",
                vec![
                    ("block", Id(block.uuid())),
                    ("state", Field::Word(state)),
                    (
                        "percent",
                        percent.map_or(Field::Absent, |p| Field::Num(p.into())),
                    ),
                ],
            )
        }
        E::Notification {
            block,
            ref title,
            ref body,
        } => (
            "notification",
            vec![
                ("block", Id(block.uuid())),
                ("title", Text(title.clone())),
                ("body", Text(body.clone())),
            ],
        ),
    }
}

#[cfg(test)]
mod tests {
    use rusty_mux_core::{BlockId, SessionId, Size, WindowId};

    use super::*;

    fn id(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    #[test]
    fn events_render_as_text_and_json() {
        let event = Event::BlockCreated {
            block: BlockId::from_uuid(id(1)),
            window: WindowId::from_uuid(id(2)),
            size: Size::new(80, 24).unwrap(),
        };
        let text = event_line(7, &event, false);
        assert_eq!(
            text,
            format!(
                "7 block-created block={} window={} cols=80 rows=24",
                id(1),
                id(2)
            )
        );
        let json = Value::from_json_str(&event_line(7, &event, true)).unwrap();
        let Value::Object(map) = json else {
            panic!("an object")
        };
        assert_eq!(map.get("seq"), Some(&Value::from(7u64)));
        assert_eq!(map.get("kind"), Some(&Value::from("block-created")));
        assert_eq!(map.get("cols"), Some(&Value::from(80u64)));
    }

    #[test]
    fn a_free_lease_has_no_holder() {
        let event = Event::LeaseChanged {
            block: BlockId::from_uuid(id(1)),
            holder: None,
        };
        assert!(event_line(1, &event, false).ends_with("holder=-"));
        assert!(event_line(1, &event, true).contains("\"holder\":null"));
    }

    #[test]
    fn program_events_render_their_details() {
        let block = BlockId::from_uuid(id(1));
        let line = |event: Event, json: bool| event_line(1, &event, json);
        let killed = Event::ProgramExited {
            block,
            status: ExitStatus::Signal(9),
        };
        assert!(line(killed.clone(), false).ends_with("code=- signal=9"));
        let json = line(killed, true);
        assert!(json.contains("\"code\":null") && json.contains("\"signal\":9"));
        let exited = Event::ProgramExited {
            block,
            status: ExitStatus::Code(-1),
        };
        assert!(line(exited, false).ends_with("code=-1 signal=-"));
        let busy = Event::ProgressChanged {
            block,
            progress: Progress::Indeterminate,
        };
        assert!(line(busy, false).ends_with("state=indeterminate percent=-"));
        let note = Event::Notification {
            block,
            title: String::new(),
            body: "a\nb".into(),
        };
        assert!(line(note.clone(), false).ends_with("title=\"\" body=\"a\\nb\""));
        assert!(line(note, true).contains("\"body\":\"a\\nb\""));
    }

    #[test]
    fn status_events_name_their_block() {
        let event = Event::BlockStatus {
            block: BlockId::from_uuid(id(1)),
        };
        assert_eq!(
            event_line(3, &event, false),
            format!("3 block-status block={}", id(1))
        );
    }

    #[test]
    fn rules_can_name_every_event_and_its_fields() {
        let (session, window, block) = (
            SessionId::from_uuid(id(1)),
            WindowId::from_uuid(id(2)),
            BlockId::from_uuid(id(3)),
        );
        let client = rusty_mux_core::ClientId::from_uuid(id(4));
        let size = Size::new(80, 24).unwrap();
        let events = [
            Event::SessionCreated { session },
            Event::SessionClosed { session },
            Event::WindowCreated { window, session },
            Event::WindowClosed { window },
            Event::BlockCreated {
                block,
                window,
                size,
            },
            Event::BlockResized { block, size },
            Event::BlockClosed { block },
            Event::ClientConnected { client },
            Event::ClientDisconnected { client },
            Event::Attached { client, block },
            Event::Detached { client, block },
            Event::LeaseChanged {
                block,
                holder: Some(client),
            },
            Event::BlockStatus { block },
            Event::PwdChanged { block },
            Event::TitleChanged { block },
            Event::ProgramExited {
                block,
                status: ExitStatus::Code(3),
            },
            Event::Bell { block },
            Event::ProgressChanged {
                block,
                progress: Progress::Paused(7),
            },
            Event::Notification {
                block,
                title: "t".into(),
                body: "b".into(),
            },
        ];
        assert_eq!(events.len(), rusty_mux_script::EVENTS.len());
        for (event, (kind, declared)) in events.iter().zip(rusty_mux_script::EVENTS) {
            let (found, values) = event_fields(event);
            assert_eq!(found, *kind);
            let names: Vec<&str> = values.iter().map(|(n, _)| n.as_str()).collect();
            // `watch` reads the new directory or title from the block, and
            // an exit has a code or a signal.
            let missing: &[&str] = match *kind {
                "pwd-changed" => &["pwd"],
                "title-changed" => &["title"],
                "program-exited" => &["signal"],
                _ => &[],
            };
            let expected: Vec<&str> = declared
                .iter()
                .copied()
                .filter(|f| !missing.contains(f))
                .collect();
            assert_eq!(names, expected, "{kind}");
        }
        let free = Event::LeaseChanged {
            block,
            holder: None,
        };
        assert_eq!(
            event_fields(&free).1,
            [("block".to_owned(), id(3).to_string())]
        );
    }

    #[test]
    fn entries_render_as_text_and_json() {
        let block = Entry::Block {
            block: BlockId::from_uuid(id(1)),
            window: WindowId::from_uuid(id(2)),
            size: Size::new(80, 24).unwrap(),
            holder: None,
            title: "vim".into(),
        };
        assert_eq!(
            entry_line(&block, false),
            format!(
                "block {} window={} cols=80 rows=24 holder=- title=\"vim\"",
                id(1),
                id(2)
            )
        );
        let Value::Object(map) = Value::from_json_str(&entry_line(&block, true)).unwrap() else {
            panic!("an object")
        };
        assert_eq!(map.get("kind"), Some(&Value::from("block")));
        assert_eq!(map.get("block"), Some(&Value::from(id(1).to_string())));
        assert_eq!(map.get("holder"), Some(&Value::Null));
        assert_eq!(map.get("title"), Some(&Value::from("vim")));
    }

    #[test]
    fn names_cannot_forge_lines_or_reach_the_terminal() {
        let name = "a b\nsession x\r\u{1b}]0;pwn\u{7}\"";
        let session = Entry::Session {
            session: SessionId::from_uuid(id(1)),
            name: name.into(),
        };
        let text = entry_line(&session, false);
        assert_eq!(
            text,
            format!(
                "session {} name=\"a b\\nsession x\\r\\u{{1b}}]0;pwn\\u{{7}}\\\"\"",
                id(1)
            )
        );
        let json = entry_line(&session, true);
        assert!(
            !text.chars().chain(json.chars()).any(char::is_control),
            "{json}"
        );
        let Value::Object(map) = Value::from_json_str(&json).unwrap() else {
            panic!("an object")
        };
        assert_eq!(map.get("name"), Some(&Value::from(name)));
    }

    #[test]
    fn captured_text_is_printed_as_is_or_wrapped() {
        assert_eq!(text_line("terminal.format", "a\n b".into(), false), "a\n b");
        let json = text_line("terminal.format", "a\n b".into(), true);
        assert!(json.contains("\"text\":\"a\\n b\""), "{json}");
    }

    #[test]
    fn requests_render_their_outcome() {
        assert_eq!(done_line("detach", None, false), None);
        assert_eq!(
            done_line("create-session", Some(id(3)), false),
            Some(id(3).to_string())
        );
        let json = done_line("create-session", Some(id(3)), true).unwrap();
        assert!(json.contains("\"ok\":true") && json.contains(&id(3).to_string()));
        assert!(failed_line("input", "no lease").contains("\"error\":\"no lease\""));
        let cursor = Cursor {
            epoch: id(4),
            seq: 9,
        };
        assert_eq!(cursor_line(cursor, false), format!("cursor {}:9", id(4)));
    }
}

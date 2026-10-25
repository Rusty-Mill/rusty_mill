//! Output lines: plain text for people, one JSON object per line with
//! `--json`.

use rusty_json::{Map, Value};
use rusty_mux_core::Event;
use rusty_mux_proto::Cursor;
use rusty_uuid::Uuid;

/// An event field's value.
enum Field {
    Id(Uuid),
    Num(u64),
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

pub(crate) fn event_line(seq: u64, event: &Event, json: bool) -> String {
    let (kind, fields) = describe(event);
    if json {
        let head = [("seq", Value::from(seq)), ("kind", Value::from(kind))];
        return object(
            head.into_iter()
                .chain(fields.into_iter().map(|(k, f)| (k, f.json()))),
        );
    }
    let mut line = format!("{seq} {kind}");
    for (name, field) in fields {
        line.push_str(&format!(" {name}={}", field.text()));
    }
    line
}

impl Field {
    fn json(self) -> Value {
        match self {
            Self::Id(id) => Value::from(id.to_string()),
            Self::Num(n) => Value::from(n),
            Self::Absent => Value::Null,
        }
    }

    fn text(self) -> String {
        match self {
            Self::Id(id) => id.to_string(),
            Self::Num(n) => n.to_string(),
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

fn describe(event: &Event) -> (&'static str, Vec<(&'static str, Field)>) {
    use Event as E;
    use Field::{Id, Num};
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
        } => (
            "block-created",
            vec![
                ("block", Id(block.uuid())),
                ("window", Id(window.uuid())),
                ("cols", Num(size.cols().into())),
                ("rows", Num(size.rows().into())),
            ],
        ),
        E::BlockResized { block, size } => (
            "block-resized",
            vec![
                ("block", Id(block.uuid())),
                ("cols", Num(size.cols().into())),
                ("rows", Num(size.rows().into())),
            ],
        ),
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
    }
}

#[cfg(test)]
mod tests {
    use rusty_mux_core::{BlockId, Size, WindowId};

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

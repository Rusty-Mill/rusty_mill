//! `overview`: the blocks of every session grouped by the card their agent
//! works on (D336), so one look shows who is on what. It covers one session
//! (the one it runs in); outside any session, every session.

use rusty_json::Value;
use rusty_mux_core::{BlockId, WindowId};
use rusty_mux_proto::{AgentMeta, Entry};
use rusty_uuid::Uuid;

/// A block as the overview shows it.
#[derive(Debug, PartialEq, Eq)]
struct Row<'a> {
    block: BlockId,
    name: &'a str,
    title: &'a str,
    exited: bool,
    held: bool,
    agent: Option<&'a AgentMeta>,
}

/// Blocks grouped by card, in order of first appearance; blocks without a
/// card come last, under `None`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Overview<'a> {
    groups: Vec<(Option<&'a str>, Vec<Row<'a>>)>,
}

impl<'a> Overview<'a> {
    pub(crate) fn of(entries: &'a [Entry], session: Option<Uuid>) -> Self {
        // A block belongs to the session of its window.
        let in_scope = |window: WindowId| {
            session.is_none_or(|s| {
                entries.iter().any(|e| {
                    matches!(e, Entry::Window { window: w, session: id, .. }
                        if *w == window && id.uuid() == s)
                })
            })
        };
        let agent = |id: BlockId| {
            entries.iter().find_map(|e| match e {
                Entry::Agent { block, meta } if *block == id => Some(meta),
                _ => None,
            })
        };
        let mut groups: Vec<(Option<&str>, Vec<Row>)> = Vec::new();
        for entry in entries {
            let Entry::Block {
                block,
                window,
                holder,
                title,
                name,
                exited,
                ..
            } = entry
            else {
                continue;
            };
            if !in_scope(*window) {
                continue;
            }
            let agent = agent(*block);
            let card = agent.map(|m| m.card.as_str()).filter(|c| !c.is_empty());
            let row = Row {
                block: *block,
                name,
                title,
                exited: *exited,
                held: holder.is_some(),
                agent,
            };
            match groups.iter_mut().find(|(c, _)| *c == card) {
                Some((_, rows)) => rows.push(row),
                None => groups.push((card, vec![row])),
            }
        }
        groups.sort_by_key(|(card, _)| card.is_none());
        Self { groups }
    }

    /// Text lines: a card, then its blocks indented under it.
    pub(crate) fn text(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for (card, rows) in &self.groups {
            let head = card.map_or("no card".to_owned(), |c| format!("card {}", quote(c)));
            lines.push(format!("{head} ({})", rows.len()));
            lines.extend(rows.iter().map(|row| format!("  {}", row.text())));
        }
        lines
    }

    /// One JSON object per card.
    pub(crate) fn json(&self) -> Vec<String> {
        self.groups
            .iter()
            .map(|(card, rows)| {
                let card = card.map_or(Value::Null, Value::from);
                let rows = rows.iter().map(Row::json).collect();
                let pairs = [("card", card), ("blocks", Value::Array(rows))];
                Value::Object(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
                    .to_json_string()
            })
            .collect()
    }
}

impl Row<'_> {
    fn state(&self) -> &'static str {
        if self.exited { "exited" } else { "running" }
    }

    fn text(&self) -> String {
        let mut line = format!("block {} {}", self.block.uuid(), self.state());
        if let Some(m) = self.agent {
            line.push_str(&format!(
                " agent={} role={}",
                quote(&m.agent),
                quote(&m.role)
            ));
        }
        if self.held {
            line.push_str(" lease=held");
        }
        if !self.name.is_empty() {
            line.push_str(&format!(" name={}", quote(self.name)));
        }
        if !self.title.is_empty() {
            line.push_str(&format!(" title={}", quote(self.title)));
        }
        line
    }

    fn json(&self) -> Value {
        let text = |s: &str| Value::from(s);
        let mut map = vec![
            ("block", Value::from(self.block.uuid().to_string())),
            ("state", text(self.state())),
            ("lease_held", Value::Bool(self.held)),
            ("name", text(self.name)),
            ("title", text(self.title)),
        ];
        if let Some(m) = self.agent {
            map.push(("agent", text(&m.agent)));
            map.push(("role", text(&m.role)));
        }
        Value::Object(map.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
    }
}

/// Quoted and escaped: a name can't add a line or reach the terminal.
fn quote(text: &str) -> String {
    format!("\"{}\"", text.escape_debug())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_mux_core::{InputMode, SessionId, Size};

    fn id(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    fn block(n: u8, name: &str, exited: bool) -> Entry {
        Entry::Block {
            block: BlockId::from_uuid(id(n)),
            window: WindowId::from_uuid(id(100)),
            size: Size::new(80, 24).unwrap(),
            holder: None,
            title: String::new(),
            name: name.into(),
            owner: None,
            input: InputMode::Open,
            exited,
        }
    }

    fn agent(n: u8, card: &str) -> Entry {
        Entry::Agent {
            block: BlockId::from_uuid(id(n)),
            meta: AgentMeta {
                agent: "claude".into(),
                role: "builder".into(),
                card: card.into(),
            },
        }
    }

    #[test]
    fn groups_blocks_under_their_card_and_plain_blocks_last() {
        let entries = [
            block(1, "shell", false),
            block(2, "", false),
            agent(2, "#7"),
            block(3, "", true),
            agent(3, "#7"),
            block(4, "", false),
            agent(4, "#8"),
        ];
        let text = Overview::of(&entries, None).text();
        assert_eq!(text.len(), 3 + 4);
        assert_eq!(text[0], "card \"#7\" (2)");
        assert!(text[1].contains("agent=\"claude\" role=\"builder\""));
        assert!(text[2].contains("exited"));
        assert_eq!(text[3], "card \"#8\" (1)");
        assert_eq!(text[5], "no card (1)");
        assert!(text[6].contains("name=\"shell\""));
    }

    #[test]
    fn a_session_scopes_it_to_the_blocks_of_its_windows() {
        let window = |n: u8, session: u8| Entry::Window {
            window: WindowId::from_uuid(id(n)),
            session: SessionId::from_uuid(id(session)),
            name: String::new(),
            layout: None,
            zoomed: None,
            focused: None,
        };
        let mut other = block(5, "elsewhere", false);
        if let Entry::Block { window, .. } = &mut other {
            *window = WindowId::from_uuid(id(101));
        }
        let entries = [
            window(100, 50),
            window(101, 51),
            block(1, "mine", false),
            other,
        ];
        let mine = Overview::of(&entries, Some(id(50))).text();
        assert_eq!(mine.len(), 2);
        assert!(mine[1].contains("mine"));
        assert_eq!(Overview::of(&entries, None).text().len(), 3);
        assert!(Overview::of(&entries, Some(id(99))).text().is_empty());
    }

    #[test]
    fn json_is_one_object_per_card_with_null_for_none() {
        let entries = [
            block(1, "shell", false),
            block(2, "", false),
            agent(2, "#7"),
        ];
        let lines = Overview::of(&entries, None).json();
        assert_eq!(lines.len(), 2);
        let first = Value::from_json_str(&lines[0]).unwrap();
        assert_eq!(first.get("card"), Some(&Value::from("#7")));
        let last = Value::from_json_str(&lines[1]).unwrap();
        assert_eq!(last.get("card"), Some(&Value::Null));
    }

    #[test]
    fn nothing_listed_prints_nothing_and_names_are_escaped() {
        assert!(Overview::of(&[], None).text().is_empty());
        let entries = [block(1, "a\nb\x1b[31m", false)];
        let text = Overview::of(&entries, None).text();
        assert_eq!(text.len(), 2);
        assert!(text[1].contains("a\\nb\\u{1b}[31m"));
    }
}

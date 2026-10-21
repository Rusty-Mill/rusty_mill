//! The card side of a hosted run.
//!
//! Orch is the only writer of assignment generations. Before any host
//! command runs under a new generation, the run records that generation with
//! the Millboard card it hosts: `card_execution_link` the first time (or to
//! replay an identical link), and the compare-and-set
//! `card_execution_recover` to move a card from the generation it carries to
//! a later one. A card already carrying a later generation means a newer
//! process holds the goal, and this one stops.
//!
//! [`HttpMillboard`] speaks Millboard's `POST /api/<op>` over `rusty_http`'s
//! blocking transport: plain `http://` only, one connection per call, no
//! async runtime and no registry dependency.

use std::fmt;
use std::net::TcpStream;
use std::time::Duration;

use orch_core::factory::ExecutionLink;
use rusty_http::body::response_framing;
use rusty_http::head::RequestHead;
use rusty_http::sync::{BodyReader, SyncTransport};
use rusty_http::{HeaderMap, Method, Url, Version};
use rusty_json::Value;

const MAX_HEAD_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

/// Where a card's execution link is kept.
pub trait CardLinks {
    /// The generation stored with `card`'s execution; `None` when the card
    /// has no execution yet.
    fn stored(&mut self, card: &str) -> Result<Option<u64>, String>;
    /// Initiate the card's execution, or replay an identical link.
    fn link(&mut self, link: &ExecutionLink) -> Result<(), String>;
    /// Move the card's execution from `expected` to `link.generation`,
    /// refused unless the card still carries `expected`.
    fn recover(&mut self, expected: u64, link: &ExecutionLink) -> Result<(), String>;
}

/// Record `link` with its card, refusing when the card already carries a
/// later generation. The board itself refuses a different execution.
pub fn publish(board: &mut dyn CardLinks, link: &ExecutionLink) -> Result<(), String> {
    match board.stored(&link.card)? {
        None => board.link(link),
        Some(stored) if stored == link.generation => board.link(link),
        Some(stored) if stored < link.generation => board.recover(stored, link),
        Some(stored) => Err(format!(
            "superseded: card {} carries generation {stored}, this run holds {}",
            link.card, link.generation
        )),
    }
}

/// A bearer token, kept out of `Debug` output.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(pub String);

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(..)")
    }
}

/// How this run reaches Millboard and who it acts as there. With a
/// per-model credential the board fixes the actor and role, so both are
/// optional; with a shared token or none, the board needs them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardAccess {
    /// `http://host[:port]`, without `/api`.
    pub url: String,
    pub token: Option<Token>,
    pub actor: Option<String>,
    pub role: Option<String>,
}

/// Millboard over its HTTP API.
#[derive(Debug)]
pub struct HttpMillboard {
    access: BoardAccess,
}

impl HttpMillboard {
    /// # Errors
    ///
    /// When `access.url` is not a plain `http://` URL.
    pub fn new(access: BoardAccess) -> Result<Self, String> {
        let url = Url::parse(&access.url).map_err(|e| format!("board url: {e}"))?;
        if url.scheme != "http" {
            return Err(format!(
                "board url: only http:// is supported, got {}://",
                url.scheme
            ));
        }
        Ok(Self { access })
    }

    fn act(&self, op: &str, mut args: Value) -> Result<Value, String> {
        if let Some(actor) = &self.access.actor {
            args.insert("actor", actor.as_str());
        }
        if let Some(role) = &self.access.role {
            args.insert("role", role.as_str());
        }
        self.call(op, &args)
    }

    fn call(&self, op: &str, args: &Value) -> Result<Value, String> {
        let fail = |e: &dyn fmt::Display| format!("millboard {op}: {e}");
        let url = Url::parse(&format!(
            "{}/api/{op}",
            self.access.url.trim_end_matches('/')
        ))
        .map_err(|e| fail(&e))?;
        let body = args.to_json_string().into_bytes();
        let stream = TcpStream::connect((url.host.as_str(), url.port)).map_err(|e| fail(&e))?;
        stream
            .set_read_timeout(Some(TIMEOUT))
            .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
            .map_err(|e| fail(&e))?;
        let mut transport = SyncTransport::new(stream);
        let mut headers = HeaderMap::new();
        let auth = self
            .access
            .token
            .as_ref()
            .map(|t| format!("Bearer {}", t.0));
        let fields = [
            ("Host", Some(url.host_header())),
            ("Content-Type", Some("application/json".to_owned())),
            ("Content-Length", Some(body.len().to_string())),
            ("Connection", Some("close".to_owned())),
            ("Authorization", auth),
        ];
        for (name, value) in fields {
            if let Some(value) = value {
                headers.insert(name, &value).map_err(|e| fail(&e))?;
            }
        }
        transport
            .write_request_head(&RequestHead {
                method: Method::Post,
                target: url.request_target(),
                version: Version::Http11,
                headers,
            })
            .and_then(|()| transport.write_body(&body))
            .map_err(|e| fail(&e))?;
        let head = transport
            .read_response_head(MAX_HEAD_BYTES)
            .map_err(|e| fail(&e))?;
        let framing =
            response_framing(&head.headers, &Method::Post, head.status).map_err(|e| fail(&e))?;
        let text = read_all(&mut transport.into_body_reader(framing)).map_err(|e| fail(&e))?;
        let reply = Value::parse(&text).map_err(|e| fail(&e))?;
        let status = head.status.as_u16();
        if (200..300).contains(&status) {
            return Ok(reply);
        }
        let why = reply
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("no error message");
        Err(fail(&format_args!("{status} {why}")))
    }
}

fn read_all(reader: &mut BodyReader<TcpStream>) -> Result<String, String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = reader.next_chunk().map_err(|e| e.to_string())? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_BODY_BYTES {
            return Err("response too large".into());
        }
    }
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

fn link_args(link: &ExecutionLink) -> Value {
    let mut args = Value::object();
    args.insert("id", link.card.as_str())
        .insert("execution", link.execution.as_str())
        .insert("goal", link.goal.as_str())
        .insert("bbp_task", link.bbp_task.as_str())
        .insert("attempt", link.attempt.as_str())
        .insert("assignment", link.assignment.as_str())
        .insert("generation", link.generation);
    args
}

impl CardLinks for HttpMillboard {
    fn stored(&mut self, card: &str) -> Result<Option<u64>, String> {
        let mut args = Value::object();
        args.insert("id", card);
        let reply = self.call("card_get", &args)?;
        let execution = reply
            .get("card")
            .ok_or("millboard card_get: no card in the reply")?
            .get("execution");
        match execution {
            None => Ok(None),
            Some(e) if e.is_null() => Ok(None),
            Some(e) => e
                .get("generation")
                .and_then(Value::as_u64)
                .map(Some)
                .ok_or_else(|| "millboard card_get: execution has no generation".to_owned()),
        }
    }

    fn link(&mut self, link: &ExecutionLink) -> Result<(), String> {
        self.act("card_execution_link", link_args(link)).map(drop)
    }

    fn recover(&mut self, expected: u64, link: &ExecutionLink) -> Result<(), String> {
        let mut args = link_args(link);
        args.insert("expected_generation", expected);
        self.act("card_execution_recover", args).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orch_core::factory::ExecutionState;

    /// A board holding links with Millboard's rules.
    #[derive(Default)]
    struct Board {
        stored: Option<ExecutionLink>,
        calls: Vec<String>,
    }

    fn same(a: &ExecutionLink, b: &ExecutionLink) -> bool {
        (
            &a.execution,
            &a.goal,
            &a.bbp_task,
            &a.attempt,
            &a.assignment,
        ) == (
            &b.execution,
            &b.goal,
            &b.bbp_task,
            &b.attempt,
            &b.assignment,
        )
    }

    impl CardLinks for Board {
        fn stored(&mut self, _: &str) -> Result<Option<u64>, String> {
            Ok(self.stored.as_ref().map(|l| l.generation))
        }
        fn link(&mut self, link: &ExecutionLink) -> Result<(), String> {
            self.calls.push(format!("link {}", link.generation));
            match &self.stored {
                Some(s) if s != link => Err("card already has a logical execution".into()),
                _ => {
                    self.stored = Some(link.clone());
                    Ok(())
                }
            }
        }
        fn recover(&mut self, expected: u64, link: &ExecutionLink) -> Result<(), String> {
            self.calls
                .push(format!("recover {expected}->{}", link.generation));
            match &self.stored {
                Some(s) if same(s, link) && s.generation == expected => {
                    self.stored = Some(link.clone());
                    Ok(())
                }
                _ => Err("refused".into()),
            }
        }
    }

    fn at(generation: u64) -> ExecutionLink {
        ExecutionLink {
            card: "card".into(),
            execution: "execution".into(),
            goal: "1".into(),
            bbp_task: "namespace".into(),
            attempt: "1".into(),
            assignment: "goal-orchestrator".into(),
            generation,
            state: ExecutionState::Running,
        }
    }

    #[test]
    fn links_once_replays_and_recovers_to_later_generations() {
        let mut board = Board::default();
        publish(&mut board, &at(1)).expect("link");
        publish(&mut board, &at(1)).expect("replay");
        publish(&mut board, &at(3)).expect("recover");
        assert_eq!(board.calls, ["link 1", "link 1", "recover 1->3"]);
        assert_eq!(board.stored, Some(at(3)));
    }

    #[test]
    fn a_card_at_a_later_generation_refuses_the_older_run() {
        let mut board = Board {
            stored: Some(at(4)),
            ..Board::default()
        };
        let error = publish(&mut board, &at(3)).expect_err("superseded");
        assert!(error.starts_with("superseded"), "{error}");
        assert!(board.calls.is_empty());
        assert_eq!(board.stored, Some(at(4)));
    }

    #[test]
    fn another_execution_on_the_card_is_refused_by_the_board() {
        let other = ExecutionLink {
            execution: "other".into(),
            ..at(1)
        };
        let mut board = Board {
            stored: Some(other.clone()),
            ..Board::default()
        };
        assert!(publish(&mut board, &at(1)).is_err());
        assert!(publish(&mut board, &at(2)).is_err());
        assert_eq!(board.stored, Some(other));
    }

    #[test]
    fn only_plain_http_boards_are_accepted() {
        let access = |url: &str| BoardAccess {
            url: url.into(),
            token: Some(Token("secret".into())),
            actor: None,
            role: None,
        };
        assert!(HttpMillboard::new(access("http://127.0.0.1:8787")).is_ok());
        let error = HttpMillboard::new(access("https://board")).expect_err("https");
        assert!(error.contains("only http://"), "{error}");
        assert!(!format!("{:?}", access("http://x")).contains("secret"));
    }
}

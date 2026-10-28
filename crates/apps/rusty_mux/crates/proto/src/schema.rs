//! The request catalog: every client request, its parameters, and what it
//! returns.
//!
//! Front ends are generated from [`CATALOG`] rather than written per request:
//! a CLI maps each [`Method`] to a subcommand and each [`Param`] to a flag, an
//! MCP server maps them to a tool and its input schema. Both parse text with
//! [`Ty::parse`] and build the wire message with [`Method::request`], so a
//! request added to [`Message`] needs one entry here and nothing else.
//!
//! Methods of a block type are named `TYPE.verb`, such as `terminal.format`,
//! and travel as one generic [`Message::BlockCall`] that the server
//! dispatches on the block's type, so a new block method needs no new
//! message. Their first parameter is the `block`.
//!
//! ```
//! use rusty_mux_proto::schema::{self, Value};
//!
//! let method = schema::method("create-session").expect("in the catalog");
//! let name = method.params[0].ty.parse("main")?;
//! let message = method.request(7, &[("name", name)])?;
//! assert_eq!(
//!     message,
//!     rusty_mux_proto::Message::CreateSession { req: 7, name: "main".into() },
//! );
//! # Ok::<(), schema::CallError>(())
//! ```

use std::fmt;

use rusty_mux_core::{BlockId, ClientId, EntityId, SessionId, Side, Size, WindowId};
use rusty_uuid::Uuid;

use crate::{Features, Launch, Message, Program};

/// Which entity an id names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entity {
    Session,
    Window,
    Block,
    Client,
}

impl Entity {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Window => "window",
            Self::Block => "block",
            Self::Client => "client",
        }
    }

    /// The id `uuid` as this kind's.
    pub const fn id(self, uuid: Uuid) -> EntityId {
        match self {
            Self::Session => EntityId::Session(SessionId::from_uuid(uuid)),
            Self::Window => EntityId::Window(WindowId::from_uuid(uuid)),
            Self::Block => EntityId::Block(BlockId::from_uuid(uuid)),
            Self::Client => EntityId::Client(ClientId::from_uuid(uuid)),
        }
    }
}

/// A parameter's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    /// An entity id, written as a UUID.
    Id(Entity),
    /// UTF-8 text.
    Text,
    /// Raw bytes. From text they are the text's UTF-8 encoding.
    Bytes,
    /// A count of cells, at least 1.
    Cells,
    /// A count of lines, possibly 0.
    Lines,
    /// A list of UTF-8 texts. From text it is one item; a front end
    /// gathers a repeated flag into one list.
    Words,
    /// On or off. From text it is `true` or `false`.
    Flag,
}

impl Ty {
    /// Parses a front end's text argument.
    pub fn parse(self, text: &str) -> Result<Value, CallError> {
        let bad = || CallError::Unparsable {
            ty: self,
            text: text.to_owned(),
        };
        Ok(match self {
            Self::Id(_) => Value::Id(text.parse().map_err(|_| bad())?),
            Self::Text => Value::Text(text.to_owned()),
            Self::Bytes => Value::Bytes(text.as_bytes().to_vec()),
            Self::Cells => match text.parse::<u16>() {
                Ok(n) if n > 0 => Value::Cells(n),
                _ => return Err(bad()),
            },
            Self::Lines => Value::Lines(text.parse().map_err(|_| bad())?),
            Self::Words => Value::Words(vec![text.to_owned()]),
            Self::Flag => Value::Flag(match text {
                "true" => true,
                "false" => false,
                _ => return Err(bad()),
            }),
        })
    }

    /// Whether `value` has this type.
    pub fn admits(self, value: &Value) -> bool {
        matches!(
            (self, value),
            (Self::Id(_), Value::Id(_))
                | (Self::Text, Value::Text(_))
                | (Self::Bytes, Value::Bytes(_))
                | (Self::Cells, Value::Cells(_))
                | (Self::Lines, Value::Lines(_))
                | (Self::Words, Value::Words(_))
                | (Self::Flag, Value::Flag(_))
        )
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id(entity) => write!(f, "{} id", entity.name()),
            Self::Text => f.write_str("text"),
            Self::Bytes => f.write_str("bytes"),
            Self::Cells => f.write_str("cell count"),
            Self::Lines => f.write_str("line count"),
            Self::Words => f.write_str("text list"),
            Self::Flag => f.write_str("flag (true or false)"),
        }
    }
}

/// An argument's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Id(Uuid),
    Text(String),
    Bytes(Vec<u8>),
    Cells(u16),
    Lines(u32),
    Words(Vec<String>),
    Flag(bool),
}

/// One parameter of a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Param {
    /// Kebab-case, unique within its method.
    pub name: &'static str,
    pub ty: Ty,
    pub doc: &'static str,
    pub presence: Presence,
}

/// Whether a request must carry a parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Required,
    /// May be left out; when given, the connection must also have
    /// negotiated `needs`, since a server without it would ignore it.
    Optional {
        needs: Features,
    },
}

/// What a successful request returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Returns {
    /// Only success: [`Message::Done`] without an id.
    Nothing,
    /// No reply on success; [`Message::Failed`] if it fails. A caller that
    /// must know the outcome sends `sync` next and waits for its reply: a
    /// failure arrives before it.
    Unanswered,
    /// The id of the entity it created, in [`Message::Done`].
    Created(Entity),
    /// [`Message::Text`] instead of [`Message::Done`].
    Text,
    /// Zero or more [`Message::Entry`], then [`Message::Done`].
    Entries,
}

/// One client request.
#[derive(Clone, Copy)]
pub struct Method {
    /// Kebab-case, unique in the catalog.
    pub name: &'static str,
    pub doc: &'static str,
    pub params: &'static [Param],
    pub returns: Returns,
    /// The features a connection must have negotiated to send it.
    pub needs: Features,
    build: fn(u64, &Args<'_>) -> Result<Message, CallError>,
}

impl fmt::Debug for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Method")
            .field("name", &self.name)
            .field("params", &self.params)
            .field("returns", &self.returns)
            .field("needs", &self.needs)
            .finish_non_exhaustive()
    }
}

/// Methods are equal when their names are, since names are unique.
impl PartialEq for Method {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for Method {}

impl Method {
    /// Whether this is a method of a block type, named `TYPE.verb`.
    pub fn is_block_method(&self) -> bool {
        self.name.contains('.')
    }

    /// The features a connection needs to send this method with arguments
    /// named `given`: the method's own, plus those of each optional
    /// parameter given.
    pub fn needs_for<'a>(&self, given: impl IntoIterator<Item = &'a str>) -> Features {
        given
            .into_iter()
            .filter_map(|name| self.params.iter().find(|p| p.name == name))
            .fold(self.needs, |all, param| match param.presence {
                Presence::Required => all,
                Presence::Optional { needs } => all.union(needs),
            })
    }

    /// Builds the request message, numbered `req`, from named arguments.
    /// Each required parameter must appear exactly once, and an optional
    /// one at most once, with a value of its type.
    pub fn request(&self, req: u64, args: &[(&str, Value)]) -> Result<Message, CallError> {
        for (i, (name, value)) in args.iter().enumerate() {
            let param = self
                .params
                .iter()
                .find(|p| p.name == *name)
                .ok_or_else(|| CallError::UnknownParam((*name).to_owned()))?;
            if args[..i].iter().any(|(earlier, _)| earlier == name) {
                return Err(CallError::Repeated(param.name));
            }
            if !param.ty.admits(value) {
                return Err(CallError::WrongType {
                    param: param.name,
                    ty: param.ty,
                });
            }
        }
        if let Some(missing) = self
            .params
            .iter()
            .filter(|p| p.presence == Presence::Required)
            .find(|p| !args.iter().any(|(name, _)| *name == p.name))
        {
            return Err(CallError::Missing(missing.name));
        }
        (self.build)(req, &Args(args))
    }
}

/// Why arguments could not become a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    UnknownParam(String),
    Missing(&'static str),
    Repeated(&'static str),
    WrongType {
        param: &'static str,
        ty: Ty,
    },
    Unparsable {
        ty: Ty,
        text: String,
    },
    /// The values are well typed but rejected together, such as a zero size.
    Invalid {
        param: &'static str,
        why: String,
    },
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownParam(name) => write!(f, "unknown parameter `{name}`"),
            Self::Missing(name) => write!(f, "missing parameter `{name}`"),
            Self::Repeated(name) => write!(f, "parameter `{name}` given twice"),
            Self::WrongType { param, ty } => write!(f, "parameter `{param}` must be a {ty}"),
            Self::Unparsable { ty, text } => write!(f, "`{text}` is not a valid {ty}"),
            Self::Invalid { param, why } => write!(f, "parameter `{param}`: {why}"),
        }
    }
}

impl std::error::Error for CallError {}

/// The method named `name`.
pub fn method(name: &str) -> Option<&'static Method> {
    CATALOG.iter().find(|m| m.name == name)
}

/// The block method named `name`: one namespaced by its block type, such
/// as `terminal.format`, sent as a [`Message::BlockCall`].
pub fn block_method(name: &str) -> Option<&'static Method> {
    method(name).filter(|m| m.is_block_method())
}

/// A [`Message::BlockCall`] of `method` with every argument but the block.
fn block_call(req: u64, a: &Args<'_>, method: &'static str) -> Result<Message, CallError> {
    Ok(Message::BlockCall {
        req,
        block: a.block()?,
        method: method.to_owned(),
        args: a
            .0
            .iter()
            .filter(|(name, _)| *name != BLOCK.name)
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    })
}

/// A [`Message::Rename`] of the `entity` its parameter of that name holds.
fn rename(req: u64, a: &Args<'_>, entity: Entity) -> Result<Message, CallError> {
    Ok(Message::Rename {
        req,
        target: entity.id(a.uuid(entity.name(), entity)?),
        name: a.text("name")?,
    })
}

/// Arguments already checked against their method's parameters, so lookups
/// of a declared parameter find a value of its type.
struct Args<'a>(&'a [(&'a str, Value)]);

impl Args<'_> {
    fn get(&self, name: &'static str) -> Result<&Value, CallError> {
        self.0
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v)
            .ok_or(CallError::Missing(name))
    }

    fn uuid(&self, name: &'static str, entity: Entity) -> Result<Uuid, CallError> {
        match self.get(name)? {
            Value::Id(id) => Ok(*id),
            _ => Err(self.wrong(name, Ty::Id(entity))),
        }
    }

    fn text(&self, name: &'static str) -> Result<String, CallError> {
        match self.get(name)? {
            Value::Text(text) => Ok(text.clone()),
            _ => Err(self.wrong(name, Ty::Text)),
        }
    }

    fn bytes(&self, name: &'static str) -> Result<Vec<u8>, CallError> {
        match self.get(name)? {
            Value::Bytes(bytes) => Ok(bytes.clone()),
            _ => Err(self.wrong(name, Ty::Bytes)),
        }
    }

    fn opt_text(&self, name: &'static str) -> Result<Option<String>, CallError> {
        self.has(name).then(|| self.text(name)).transpose()
    }

    fn has(&self, name: &str) -> bool {
        self.0.iter().any(|(n, _)| *n == name)
    }

    /// The program `command` and `arg` name; arguments need a command.
    fn launch(&self) -> Result<Launch, CallError> {
        let args = match self.has("arg") {
            false => Vec::new(),
            true => match self.get("arg")? {
                Value::Words(words) => words.clone(),
                _ => return Err(self.wrong("arg", Ty::Words)),
            },
        };
        let program = match self.opt_text("command")? {
            Some(command) => Some(Program { command, args }),
            None if args.is_empty() => None,
            None => {
                return Err(CallError::Invalid {
                    param: "arg",
                    why: "arguments need a `command`".into(),
                });
            }
        };
        Ok(Launch {
            program,
            cwd: self.opt_text("cwd")?,
        })
    }

    fn size(&self) -> Result<Size, CallError> {
        let cells = |name| match self.get(name)? {
            Value::Cells(n) => Ok(*n),
            _ => Err(self.wrong(name, Ty::Cells)),
        };
        let (cols, rows) = (cells("cols")?, cells("rows")?);
        Size::new(cols, rows).map_err(|e| CallError::Invalid {
            param: if cols == 0 { "cols" } else { "rows" },
            why: e.to_string(),
        })
    }

    /// The `side` parameter: `left`, `right`, `up` or `down`.
    fn side(&self) -> Result<Side, CallError> {
        let text = self.text("side")?;
        text.parse().map_err(|_| CallError::Invalid {
            param: "side",
            why: format!("`{text}` is not left, right, up or down"),
        })
    }

    fn block(&self) -> Result<BlockId, CallError> {
        self.uuid("block", Entity::Block).map(BlockId::from_uuid)
    }

    fn wrong(&self, param: &'static str, ty: Ty) -> CallError {
        CallError::WrongType { param, ty }
    }
}

/// A parameter every request must carry.
const fn required(name: &'static str, ty: Ty, doc: &'static str) -> Param {
    Param {
        name,
        ty,
        doc,
        presence: Presence::Required,
    }
}

/// An optional parameter of a block's [`Launch`].
const fn launch(name: &'static str, ty: Ty, doc: &'static str) -> Param {
    Param {
        name,
        ty,
        doc,
        presence: Presence::Optional {
            needs: Features::LAUNCH,
        },
    }
}

/// An optional parameter of `terminal.format`.
const fn capture(name: &'static str, doc: &'static str) -> Param {
    Param {
        name,
        ty: Ty::Flag,
        doc,
        presence: Presence::Optional {
            needs: Features::CAPTURE,
        },
    }
}

const BLOCK: Param = required("block", Ty::Id(Entity::Block), "The block to act on.");
const NEW_NAME: Param = required(
    "name",
    Ty::Text,
    "The new name, at most 256 bytes; empty clears it.",
);
const COMMAND: Param = launch(
    "command",
    Ty::Text,
    "The program to run instead of the shell; a bare name is looked up in the server's PATH.",
);
const ARG: Param = launch(
    "arg",
    Ty::Words,
    "The command's arguments, in order; repeat the flag for each.",
);
const CWD: Param = launch(
    "cwd",
    Ty::Text,
    "The absolute path to start in, instead of the server's directory.",
);
const COLS: Param = required("cols", Ty::Cells, "Width in cells.");
const ROWS: Param = required("rows", Ty::Cells, "Height in cells.");

/// Every client request except the [`Message::Hello`] handshake.
pub static CATALOG: &[Method] = &[
    Method {
        name: "create-session",
        doc: "Create a named session.",
        params: &[required(
            "name",
            Ty::Text,
            "The session's name, at most 256 bytes.",
        )],
        needs: Features::NONE,
        returns: Returns::Created(Entity::Session),
        build: |req, a| {
            Ok(Message::CreateSession {
                req,
                name: a.text("name")?,
            })
        },
    },
    Method {
        name: "create-window",
        doc: "Create a named window in a session.",
        params: &[
            required(
                "session",
                Ty::Id(Entity::Session),
                "The session to add the window to.",
            ),
            required("name", Ty::Text, "The window's name, at most 256 bytes."),
        ],
        needs: Features::NONE,
        returns: Returns::Created(Entity::Window),
        build: |req, a| {
            Ok(Message::CreateWindow {
                req,
                session: SessionId::from_uuid(a.uuid("session", Entity::Session)?),
                name: a.text("name")?,
            })
        },
    },
    Method {
        name: "create-block",
        doc: "Start a program, the server's shell unless `command` names one, in a new block of a window.",
        params: &[
            required(
                "window",
                Ty::Id(Entity::Window),
                "The window to add the block to.",
            ),
            COLS,
            ROWS,
            COMMAND,
            ARG,
            CWD,
        ],
        needs: Features::NONE,
        returns: Returns::Created(Entity::Block),
        build: |req, a| {
            Ok(Message::CreateBlock {
                req,
                window: WindowId::from_uuid(a.uuid("window", Entity::Window)?),
                size: a.size()?,
                launch: a.launch()?,
            })
        },
    },
    Method {
        name: "attach",
        doc: "Receive the block's screen, then its output.",
        params: &[BLOCK],
        needs: Features::CHECKPOINT_V5,
        returns: Returns::Nothing,
        build: |req, a| {
            Ok(Message::Attach {
                req,
                block: a.block()?,
            })
        },
    },
    Method {
        name: "detach",
        doc: "Stop receiving the attached block's output.",
        params: &[],
        needs: Features::NONE,
        returns: Returns::Nothing,
        build: |req, _| Ok(Message::Detach { req }),
    },
    Method {
        name: "acquire-lease",
        doc: "Become the block's only writer of input and size.",
        params: &[BLOCK],
        needs: Features::NONE,
        returns: Returns::Nothing,
        build: |req, a| {
            Ok(Message::AcquireLease {
                req,
                block: a.block()?,
            })
        },
    },
    Method {
        name: "release-lease",
        doc: "Give up the block's input lease.",
        params: &[BLOCK],
        needs: Features::NONE,
        returns: Returns::Nothing,
        build: |req, a| {
            Ok(Message::ReleaseLease {
                req,
                block: a.block()?,
            })
        },
    },
    Method {
        name: "input",
        doc: "Send bytes to the block's program, as if typed. Needs the lease.",
        params: &[BLOCK, required("data", Ty::Bytes, "The bytes to send.")],
        needs: Features::NONE,
        returns: Returns::Unanswered,
        build: |req, a| {
            Ok(Message::Input {
                req,
                block: a.block()?,
                data: a.bytes("data")?,
            })
        },
    },
    Method {
        name: "resize",
        doc: "Change the block's size. Needs the lease.",
        params: &[BLOCK, COLS, ROWS],
        needs: Features::NONE,
        returns: Returns::Nothing,
        build: |req, a| {
            Ok(Message::Resize {
                req,
                block: a.block()?,
                size: a.size()?,
            })
        },
    },
    Method {
        name: "sync",
        doc: "Change nothing; the reply means every earlier request was handled.",
        params: &[],
        needs: Features::SYNC,
        returns: Returns::Nothing,
        build: |req, _| Ok(Message::Sync { req }),
    },
    Method {
        name: "rename-session",
        doc: "Rename a session.",
        params: &[
            required("session", Ty::Id(Entity::Session), "The session to rename."),
            NEW_NAME,
        ],
        needs: Features::LABELS,
        returns: Returns::Nothing,
        build: |req, a| rename(req, a, Entity::Session),
    },
    Method {
        name: "rename-window",
        doc: "Rename a window.",
        params: &[
            required("window", Ty::Id(Entity::Window), "The window to rename."),
            NEW_NAME,
        ],
        needs: Features::LABELS,
        returns: Returns::Nothing,
        build: |req, a| rename(req, a, Entity::Window),
    },
    Method {
        name: "rename-block",
        doc: "Name a block, so a target can name it.",
        params: &[
            required("block", Ty::Id(Entity::Block), "The block to rename."),
            NEW_NAME,
        ],
        needs: Features::LABELS,
        returns: Returns::Nothing,
        build: |req, a| rename(req, a, Entity::Block),
    },
    Method {
        name: "rename-client",
        doc: "Name a client connection, such as an attached terminal.",
        params: &[
            required("client", Ty::Id(Entity::Client), "The client to rename."),
            NEW_NAME,
        ],
        needs: Features::LABELS,
        returns: Returns::Nothing,
        build: |req, a| rename(req, a, Entity::Client),
    },
    Method {
        name: "disconnect-client",
        doc: "End another client's connection: it is detached and gives up its input lease. \
              It may connect again.",
        params: &[required(
            "client",
            Ty::Id(Entity::Client),
            "The client to disconnect; not the caller's own.",
        )],
        needs: Features::LABELS,
        returns: Returns::Nothing,
        build: |req, a| {
            Ok(Message::Disconnect {
                req,
                client: ClientId::from_uuid(a.uuid("client", Entity::Client)?),
            })
        },
    },
    Method {
        name: "split-block",
        doc: "Start a program, the server's shell unless `command` names one, in a new block \
              beside another, splitting its place in half; unzooms the window.",
        params: &[
            required(
                "block",
                Ty::Id(Entity::Block),
                "The block to split; the new one takes part of its size.",
            ),
            required(
                "side",
                Ty::Text,
                "Where the new block goes: `left`, `right`, `up` or `down`.",
            ),
            COMMAND,
            ARG,
            CWD,
        ],
        needs: Features::LAYOUT,
        returns: Returns::Created(Entity::Block),
        build: |req, a| {
            Ok(Message::Split {
                req,
                block: a.block()?,
                side: a.side()?,
                launch: a.launch()?,
            })
        },
    },
    Method {
        name: "zoom-block",
        doc: "Show a block alone over its whole window, until it is unzoomed or closed, \
              or a block is split off in the window.",
        params: &[required(
            "block",
            Ty::Id(Entity::Block),
            "The block to zoom.",
        )],
        needs: Features::LAYOUT,
        returns: Returns::Nothing,
        build: |req, a| {
            Ok(Message::Zoom {
                req,
                block: a.block()?,
            })
        },
    },
    Method {
        name: "unzoom-window",
        doc: "Show every block of a window again.",
        params: &[required(
            "window",
            Ty::Id(Entity::Window),
            "The window to unzoom.",
        )],
        needs: Features::LAYOUT,
        returns: Returns::Nothing,
        build: |req, a| {
            Ok(Message::Unzoom {
                req,
                window: WindowId::from_uuid(a.uuid("window", Entity::Window)?),
            })
        },
    },
    Method {
        name: "list",
        doc: "List every session, window, block and client.",
        params: &[],
        needs: Features::INSPECT,
        returns: Returns::Entries,
        build: |req, _| Ok(Message::List { req }),
    },
    Method {
        name: "terminal.format",
        doc: "Read the terminal's screen as text, after some lines of its history: one line \
              per row, without trailing blanks.",
        params: &[
            BLOCK,
            required(
                "history",
                Ty::Lines,
                "How many scrollback lines above the screen to include.",
            ),
            capture(
                "trim",
                "Leave out the blank lines before the first and after the last line with text.",
            ),
            capture(
                "unwrap",
                "Join rows the program's text wrapped onto into one line, keeping their blanks.",
            ),
            capture(
                "vt",
                "Keep colors and attributes as SGR escape sequences, each line starting from \
                 the default style and returning to it.",
            ),
        ],
        needs: Features::BLOCK_CALL,
        returns: Returns::Text,
        build: |req, a| block_call(req, a, "terminal.format"),
    },
    Method {
        name: "terminal.send-keys",
        doc: "Press keys in the terminal, in order, encoded as its program expects them. \
              Needs the lease.",
        params: &[
            BLOCK,
            required(
                "key",
                Ty::Words,
                "A key: a character, or one of enter, tab, space, backspace, escape, up, down, \
                 left, right, home, end, page-up, page-down, insert, delete, f1 to f12; after \
                 any of `ctrl+`, `alt+` and `shift+`, as in `ctrl+c`. Repeat the flag for each.",
            ),
        ],
        needs: Features::BLOCK_CALL,
        returns: Returns::Unanswered,
        build: |req, a| block_call(req, a, "terminal.send-keys"),
    },
    Method {
        name: "terminal.pwd",
        doc: "Read the working directory of the terminal's program.",
        params: &[BLOCK],
        needs: Features::BLOCK_CALL,
        returns: Returns::Text,
        build: |req, a| block_call(req, a, "terminal.pwd"),
    },
    Method {
        name: "terminal.title",
        doc: "Read the title the terminal's program last set.",
        params: &[BLOCK],
        needs: Features::BLOCK_CALL,
        returns: Returns::Text,
        build: |req, a| block_call(req, a, "terminal.title"),
    },
    Method {
        name: "terminal.status",
        doc: "Read what the terminal's programs report doing with OSC 7501: one \
              report body per line, the root record first.",
        params: &[BLOCK],
        needs: Features::BLOCK_CALL,
        returns: Returns::Text,
        build: |req, a| block_call(req, a, "terminal.status"),
    },
    Method {
        name: "terminal.process",
        doc: "Read what runs in the terminal: one process per line as `PID<TAB>NAME<TAB>COMMAND \
              LINE`, the foreground process first, then its ancestors, the block's own program \
              last. A backslash, tab, newline or carriage return in a field is written `\\\\`, \
              `\\t`, `\\n` or `\\r`, any other control character `\\u{HEX}`. A name or command \
              line the platform does not tell is empty.",
        params: &[BLOCK],
        needs: Features::BLOCK_CALL,
        returns: Returns::Text,
        build: |req, a| block_call(req, a, "terminal.process"),
    },
    Method {
        name: "close-block",
        doc: "Stop the block's program and remove the block.",
        params: &[BLOCK],
        needs: Features::NONE,
        returns: Returns::Nothing,
        build: |req, a| {
            Ok(Message::CloseBlock {
                req,
                block: a.block()?,
            })
        },
    },
];

#[cfg(test)]
mod tests;

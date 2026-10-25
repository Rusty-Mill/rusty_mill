//! The request catalog: every client request, its parameters, and what it
//! returns.
//!
//! Front ends are generated from [`CATALOG`] rather than written per request:
//! a CLI maps each [`Method`] to a subcommand and each [`Param`] to a flag, an
//! MCP server maps them to a tool and its input schema. Both parse text with
//! [`Ty::parse`] and build the wire message with [`Method::request`], so a
//! request added to [`Message`] needs one entry here and nothing else.
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

use rusty_mux_core::{BlockId, SessionId, Size, WindowId};
use rusty_uuid::Uuid;

use crate::Message;

/// Which entity an id names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entity {
    Session,
    Window,
    Block,
}

impl Entity {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Window => "window",
            Self::Block => "block",
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
}

/// One parameter of a request. Every parameter is required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Param {
    /// Kebab-case, unique within its method.
    pub name: &'static str,
    pub ty: Ty,
    pub doc: &'static str,
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
}

/// One client request.
#[derive(Clone, Copy)]
pub struct Method {
    /// Kebab-case, unique in the catalog.
    pub name: &'static str,
    pub doc: &'static str,
    pub params: &'static [Param],
    pub returns: Returns,
    build: fn(u64, &Args<'_>) -> Result<Message, CallError>,
}

impl fmt::Debug for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Method")
            .field("name", &self.name)
            .field("params", &self.params)
            .field("returns", &self.returns)
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
    /// Builds the request message, numbered `req`, from named arguments.
    /// Each parameter must appear exactly once, with a value of its type.
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

    fn block(&self) -> Result<BlockId, CallError> {
        self.uuid("block", Entity::Block).map(BlockId::from_uuid)
    }

    fn wrong(&self, param: &'static str, ty: Ty) -> CallError {
        CallError::WrongType { param, ty }
    }
}

const BLOCK: Param = Param {
    name: "block",
    ty: Ty::Id(Entity::Block),
    doc: "The block to act on.",
};
const COLS: Param = Param {
    name: "cols",
    ty: Ty::Cells,
    doc: "Width in cells.",
};
const ROWS: Param = Param {
    name: "rows",
    ty: Ty::Cells,
    doc: "Height in cells.",
};

/// Every client request except the [`Message::Hello`] handshake.
pub static CATALOG: &[Method] = &[
    Method {
        name: "create-session",
        doc: "Create a named session.",
        params: &[Param {
            name: "name",
            ty: Ty::Text,
            doc: "The session's name.",
        }],
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
            Param {
                name: "session",
                ty: Ty::Id(Entity::Session),
                doc: "The session to add the window to.",
            },
            Param {
                name: "name",
                ty: Ty::Text,
                doc: "The window's name.",
            },
        ],
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
        doc: "Start a shell in a new block of a window.",
        params: &[
            Param {
                name: "window",
                ty: Ty::Id(Entity::Window),
                doc: "The window to add the block to.",
            },
            COLS,
            ROWS,
        ],
        returns: Returns::Created(Entity::Block),
        build: |req, a| {
            Ok(Message::CreateBlock {
                req,
                window: WindowId::from_uuid(a.uuid("window", Entity::Window)?),
                size: a.size()?,
            })
        },
    },
    Method {
        name: "attach",
        doc: "Receive the block's screen, then its output.",
        params: &[BLOCK],
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
        returns: Returns::Nothing,
        build: |req, _| Ok(Message::Detach { req }),
    },
    Method {
        name: "acquire-lease",
        doc: "Become the block's only writer of input and size.",
        params: &[BLOCK],
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
        params: &[
            BLOCK,
            Param {
                name: "data",
                ty: Ty::Bytes,
                doc: "The bytes to send.",
            },
        ],
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
        returns: Returns::Nothing,
        build: |req, _| Ok(Message::Sync { req }),
    },
    Method {
        name: "close-block",
        doc: "Stop the block's program and remove the block.",
        params: &[BLOCK],
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

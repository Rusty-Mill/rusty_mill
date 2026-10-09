use std::fmt;
use std::io;

/// Why talking to RLBot core failed.
#[derive(Debug)]
pub enum Error {
    /// The socket failed.
    Io(io::Error),
    /// Core sent bytes that are not a valid message, or a message could not be encoded.
    Wire(rb_rlbot_wire::Error),
    /// Core closed the connection.
    Closed,
    /// Core sent a disconnect signal before the handshake finished.
    Disconnected,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "socket: {e}"),
            Error::Wire(e) => write!(f, "protocol: {e}"),
            Error::Closed => f.write_str("core closed the connection"),
            Error::Disconnected => f.write_str("core asked the client to disconnect"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Wire(e) => Some(e),
            Error::Closed | Error::Disconnected => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}

impl From<rb_rlbot_wire::Error> for Error {
    fn from(e: rb_rlbot_wire::Error) -> Error {
        Error::Wire(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

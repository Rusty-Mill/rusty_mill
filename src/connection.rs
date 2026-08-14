use crate::error::{Error, Result};

/// A connection to a database.
///
/// Currently supports only an in-memory backend with no query execution —
/// the storage and execution engine are tracked as `parity-gap` issues.
/// See `ARCHITECTURE.md` for the intended engine/API boundary.
pub struct Connection {
    open: bool,
}

impl Connection {
    /// Opens a new in-memory connection.
    pub fn open_in_memory() -> Result<Connection> {
        Ok(Connection { open: true })
    }

    /// Returns whether the connection is still open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Closes the connection.
    pub fn close(mut self) -> Result<()> {
        if !self.open {
            return Err(Error::ConnectionClosed);
        }
        self.open = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_in_memory_starts_open() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(conn.is_open());
    }

    #[test]
    fn close_marks_connection_closed() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(conn.close().is_ok());
    }
}

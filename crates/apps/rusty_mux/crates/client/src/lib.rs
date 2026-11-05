//! The client side of rusty_mux's attach protocol, shared by the terminal
//! client (`rusty_mux_tui`) and the GUI client, so neither links the
//! server.
//!
//! [`conn`] moves frames over any byte stream. [`attach`] connects to a
//! server's socket, says Hello, reads the Welcome, takes a client over on a
//! fresh path and reads what a window shows from a List. Checkpoint
//! assembly and the cell style codec come from `rusty_mux_proto` and are
//! re-exported here.

#[cfg(any(target_os = "linux", windows))]
pub mod attach;
pub mod conn;

pub use conn::Conn;
pub use rusty_mux_proto::{checkpoint, style};

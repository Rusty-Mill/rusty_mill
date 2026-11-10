//! The client side of rusty_mux's attach protocol, shared by the terminal
//! client (`rusty_mux_tui`), the GUI client and `rusty-mux-ctl`, so none
//! links the server.
//!
//! [`conn`] moves frames over any byte stream. [`attach`] connects to a
//! server's socket, says Hello, reads the Welcome, takes a client over on a
//! fresh path and reads what a window shows from a List. [`older`] words the
//! notice for a server that lacks features this client asked for (C36).
//! [`ssh`] forwards a local socket to a server's over OpenSSH, for the
//! terminal client's `--ssh` and `rusty-mux-ctl`'s saved hosts (term pack
//! D194). Checkpoint assembly and the cell style codec come from
//! `rusty_mux_proto` and are re-exported here with the `screen` feature,
//! on by default; `rusty-mux-ctl` turns it off and so never builds the
//! terminal engine.

#[cfg(any(target_os = "linux", windows))]
pub mod attach;
#[cfg(any(target_os = "linux", windows))]
pub mod autostart;
pub mod conn;
pub mod ssh;

pub use conn::Conn;
pub use rusty_mux_proto::older;
#[cfg(feature = "screen")]
pub use rusty_mux_proto::{checkpoint, style};

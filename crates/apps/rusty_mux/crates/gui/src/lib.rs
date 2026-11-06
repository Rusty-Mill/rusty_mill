//! rusty_mux's GUI client: a server window drawn in rusty_term's native
//! window, through its session-source port (`rusty_term::gui::source`,
//! pack D101).
//!
//! [`keys`] names blocks by the window's pane keys, [`tree`] turns a
//! server layout into the window's. [`source`] attaches to one server
//! window: a block connection per shown block, its input lease following
//! the shared focus (pack D102).

pub mod keys;
#[cfg(any(target_os = "linux", windows))]
pub mod source;
pub mod tree;

//! rusty_mux's GUI client: a server window drawn in rusty_term's native
//! window, through its session-source port (`rusty_term::gui::source`,
//! pack D101).
//!
//! [`keys`] names blocks and windows by the window's keys, [`tree`] turns
//! a server layout into the window's, [`session`] keeps the session the
//! top strip shows, and [`headers`] what each block's header says. [`source`] attaches to a server session and shows its
//! active window: a block connection per shown block, its input lease
//! following the shared focus (pack D102).

pub mod headers;
pub mod keys;
pub mod session;
#[cfg(any(target_os = "linux", windows))]
pub mod source;
pub mod tree;

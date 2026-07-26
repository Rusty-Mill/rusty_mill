#![no_std]
#![deny(missing_docs)]

//! # `rusty_gui`
//!
//! A `#![no_std]` + `alloc` sovereign OS windowing manager, input event pump,
//! and native clipboard access layer for the **Rusty Mill** ecosystem.

extern crate alloc;

pub mod clipboard;
pub mod event;
pub mod window;

pub use clipboard::Clipboard;
pub use event::{Event, KeyCode, MouseButton};
pub use window::{Window, WindowBuilder};

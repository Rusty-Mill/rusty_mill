#![cfg_attr(not(test), no_std)]
#![deny(missing_docs)]

//! # `rusty_font`
//!
//! A `#![no_std]` + `alloc` sovereign TrueType / OpenType font table parser
//! and SIMD-accelerated glyph vector outline rasterizer for the **Rusty Mill** ecosystem.

extern crate alloc;

mod cff;
pub mod glyph;
pub mod rasterizer;
pub mod ttf;

pub use glyph::{GlyphOutline, Point};
pub use rasterizer::Rasterizer;
pub use ttf::Font;

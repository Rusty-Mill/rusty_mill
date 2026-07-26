#![no_std]
#![deny(missing_docs)]

//! # `rusty_gpu`
//!
//! A `#![no_std]` + `alloc` sovereign CPU software framebuffer presenter, SIMD-accelerated
//! 2D/3D vector rasterizer, and GPU surface presentation engine for the **Rusty Mill** ecosystem.

extern crate alloc;

pub mod color;
pub mod framebuffer;
pub mod pipeline;

pub use color::Color;
pub use framebuffer::Framebuffer;
pub use pipeline::Pipeline;

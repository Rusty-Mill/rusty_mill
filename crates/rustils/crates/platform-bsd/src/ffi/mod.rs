//! Raw bindings layer — currently the `libc` crate, curated in
//! [`libc_surface`]. Mirrors `platform-linux::ffi`'s layering.

pub mod libc_surface;
/// Security.framework / CoreFoundation bindings for the TrustAnchors
/// slice (rustils#88). macOS-only; the module gates itself.
pub mod security_framework;

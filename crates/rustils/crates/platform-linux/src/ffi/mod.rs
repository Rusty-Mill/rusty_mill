//! Raw-binding layer. Today this is a curation point over the `libc`
//! crate — a deliberately narrowed import surface, not hand-transcribed
//! declarations (RFC v2 §2: hand-copying machine-known facts teaches
//! nothing and adds transcription risk). Track P will grow a sibling
//! module of raw syscall stubs here, feature-gated, post-R2.

pub mod libc_surface;

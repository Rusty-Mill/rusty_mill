//! Harness for constant-time *evidence* on first-party crypto. Use it from
//! `[dev-dependencies]` and examples only.
//!
//! Also [`wycheproof`], a loader for Project Wycheproof JSON vector files.
//!
//! Three methods, none of which proves the absence of a leak:
//!
//! - [`taint`]: mark secret bytes undefined for valgrind's memcheck, which
//!   then reports any branch or address that depends on them (executed paths
//!   only; variable-latency instructions are invisible to it).
//! - [`timing`]: a dudect-style Welch's t-test over two input classes.
//!   "Not detected" is all a pass means.
//! - `scripts/disasm_audit.py`: counts conditional jumps and divisions in
//!   named functions of one build. Valid for that rustc, flags and target only.
//!
//! `scripts/valgrind_selftest.sh` runs the planted-leak probe so CI can show
//! the tools themselves still catch a leak.

#![deny(unsafe_code)]

pub mod taint;
pub mod timing;
pub mod wycheproof;

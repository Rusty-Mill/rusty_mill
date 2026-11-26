//! Test support for rusty_fuse backends.
//!
//! - [`MemFs`]: an in-memory, read-only-through-the-port backend, built with
//!   `add_*` calls. It is the reference the port is tested against with no OS
//!   mount (DECISIONS D12, D22).
//! - [`fixture`]: the tree every backend is loaded with before the shared
//!   assertions run.
//! - [`assert_fixture`]: the shared assertions. They panic on failure, as test
//!   helpers do.

#![forbid(unsafe_code)]

mod assertions;
mod fixture;
mod memfs;

pub use assertions::{assert_fixture, assert_symlink, collect_dir, CappedSink};
pub use fixture::{big_contents, fixture, FixtureItem, BIG_LEN, HELLO};
pub use memfs::MemFs;

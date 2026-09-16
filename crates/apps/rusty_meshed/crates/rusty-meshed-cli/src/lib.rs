//! The `meshed` operator CLI -- the Rust port of `meshed.cli`.
//!
//! See `../../capability-manifest.md` (rows CLI-001..053) for the
//! capabilities this crate covers. `health`/`lineage`/`metrics`/`slo`
//! (CLI-001..042) are implemented; the demo scripts (CLI-043..047) and
//! the docker-compose infra rows (CLI-048..053) are not yet.
//!
//! Each subcommand's business logic lives in its own module
//! ([`health`], [`lineage`], [`metrics`], [`slo`]) as a plain function
//! returning a [`command_output::CommandOutput`] (text + exit code)
//! rather than printing and calling `process::exit` directly, so tests
//! can assert on both without spawning a subprocess. `main.rs` is the
//! thin binary entry point that parses argv via [`app::Cli`], resolves
//! shared setup ([`rusty_meshed_core::PlatformConfig`], a registry
//! `Connection`), and does the actual printing/exiting.

pub mod app;
pub mod command_output;
pub mod format;
pub mod health;
pub mod lineage;
pub mod metrics;
pub mod slo;

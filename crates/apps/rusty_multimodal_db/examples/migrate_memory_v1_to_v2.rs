//! A real, runnable schema migration: reads a `memory::Memory`-tagged
//! (pre-`ADR-0056`, 12-field) `Memory` directory and writes a fresh
//! `memory::Memory@2` (14-field) directory `Memory`'s own current
//! production code opens without any special handling.
//!
//! This is `docs/design/SCHEMA-MIGRATION-DESIGN.md`'s (`ADR-0066`)
//! worked example — the answer to `ADR-0056`'s own deferred question:
//! "the day a directory cannot be re-pushed is the trigger for a
//! layout version with an in-place upgrade." No deployment needs this
//! today (`ADR-0056` records that no layout-1 directory exists), so
//! running it against a real directory is exactly what this proves is
//! now possible, not a demonstration against a fixed, hardcoded path.
//!
//! Run with: `cargo run --example migrate_memory_v1_to_v2 -- <old_path> <new_path>`
//!
//! `<old_path>` is an existing `Memory` mmap-store directory written
//! before `ADR-0056` (magic `memory::Memory`, no `deleted_at_unix_ms`/
//! `node_id`). `<new_path>` must **not** already exist — refused before
//! any write, so a mistake here can never destroy the source data. On
//! success, point a `memory_server` deployment's `SERVER_DATA_DIR` at
//! `<new_path>` (the same manual "swap the deployment path" step
//! `ADR-0065`'s `Backup` already established for restoring a backup).

#[path = "support/migrate_memory_v1_to_v2_lib.rs"]
mod migration;

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(old_path), Some(new_path), None) = (args.next(), args.next(), args.next()) else {
        eprintln!(
            "usage: migrate_memory_v1_to_v2 <old_path> <new_path>\n\n\
             Reads a pre-ADR-0056 (12-field) Memory directory at <old_path> \
             and writes a fresh, current (14-field) Memory directory at \
             <new_path>. <new_path> must not already exist."
        );
        return ExitCode::FAILURE;
    };
    let old_path = PathBuf::from(old_path);
    let new_path = PathBuf::from(new_path);

    match migration::migrate(&old_path, &new_path) {
        Ok(report) => {
            println!(
                "migrated {} record(s), {} mentions edge(s): {} -> {}",
                report.records,
                report.mentions_edges,
                old_path.display(),
                new_path.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("migration failed: {e}");
            ExitCode::FAILURE
        }
    }
}

//! Restore a real `Request::Backup` directory offline (`ADR-0070`).
//!
//! Run: `cargo run -p rusty_multimodal_db --example restore_backup --
//! <backup_dir> <target_stem> <memory|entity|relation>`.
//! Preserve the original stem (`memories.mmap`, `entities.mmap`, or
//! `relations.mmap`). Stop the destination server before restoring;
//! point `SERVER_DATA_DIR` at the restored parent and (re)start afterward.
//! Final renames are atomic per file, not as a group (`RST-FR-003`).

#[path = "support/restore_backup_lib.rs"]
mod restore_backup;

use std::path::Path;

fn usage() -> ! {
    eprintln!("usage: restore_backup <backup_dir> <target_stem> <domain: memory|entity|relation>");
    std::process::exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, backup_dir, target_stem, domain] = args.as_slice() else {
        usage();
    };
    let domain = domain
        .parse::<restore_backup::Domain>()
        .unwrap_or_else(|error| {
            eprintln!("{error}");
            usage();
        });
    let target_stem = Path::new(target_stem);
    match restore_backup::restore(Path::new(backup_dir), target_stem, domain) {
        Ok(report) => {
            let parent = target_stem
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            println!(
                "restored {} file(s), {} record(s) at {}",
                report.files,
                report.records,
                target_stem.display()
            );
            println!("Set SERVER_DATA_DIR to {} and (re)start the server binary to use the restored data.", parent.display());
        }
        Err(error) => {
            eprintln!("restore failed: {error}");
            std::process::exit(1);
        }
    }
}

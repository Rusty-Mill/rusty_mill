//! Build script: resolve the *actual* `boxcars` version from `Cargo.lock` and
//! expose it as the `BOXCARS_VERSION` compile-time env var.
//!
//! This fixes the "parser_version reported the local crate version" wart: the
//! canonical model must pin the real decoder version for reproducibility, and
//! a dependency's resolved version is not otherwise available at runtime.

use std::fs;
use std::path::Path;

fn main() {
    let version = locate_lockfile()
        .and_then(|lock| boxcars_version(&lock))
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=BOXCARS_VERSION={version}");
}

/// Find `Cargo.lock`, walking up from the manifest dir (workspaces keep it at
/// the workspace root, not the package dir).
fn locate_lockfile() -> Option<String> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let mut dir: Option<&Path> = Some(Path::new(&manifest));
    while let Some(d) = dir {
        let candidate = d.join("Cargo.lock");
        if candidate.exists() {
            println!("cargo:rerun-if-changed={}", candidate.display());
            return fs::read_to_string(candidate).ok();
        }
        dir = d.parent();
    }
    None
}

/// Pull the `version` of the `boxcars` package out of `Cargo.lock` TOML.
///
/// Avoids a TOML dependency in the build graph by scanning the simple, stable
/// `[[package]]` block layout cargo emits.
fn boxcars_version(lock: &str) -> Option<String> {
    let mut in_boxcars = false;
    for line in lock.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            in_boxcars = false;
        } else if let Some(name) = line.strip_prefix("name = ") {
            in_boxcars = name.trim_matches('"') == "boxcars";
        } else if in_boxcars {
            if let Some(ver) = line.strip_prefix("version = ") {
                return Some(ver.trim_matches('"').to_string());
            }
        }
    }
    None
}

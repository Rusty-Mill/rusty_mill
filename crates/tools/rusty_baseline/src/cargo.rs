//! Everything the baseline asks of cargo: the dependency closure, timed
//! builds, and the binary's entry-point source.

use std::collections::BTreeSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use crate::products::Product;

/// A product's dependency closure (normal and build edges, host target),
/// excluding the product's own package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Closure {
    /// Packages from a path in this workspace.
    pub workspace: usize,
    /// Packages from a registry or git.
    pub external: usize,
}

/// Runs `cargo tree` for `product` and counts its distinct packages.
pub fn closure(product: &Product) -> Result<Closure, String> {
    let mut command = cargo();
    command.args(["tree", "--locked", "-e", "normal,build", "--prefix", "none"]);
    command.args(["-p", &product.package]);
    with_features(&mut command, product);
    let stdout = run(&mut command)?;
    Ok(count_closure(&stdout))
}

/// Counts `cargo tree --prefix none` output. Each line is
/// `name vX.Y.Z [(source)] [(proc-macro)] [(*)]`; the first is the root.
/// A `(source)` that starts with `/` or a drive letter is a workspace path;
/// a git source (`(https://…)`) or none at all is external.
fn count_closure(tree: &str) -> Closure {
    let packages: BTreeSet<&str> = tree
        .lines()
        .skip(1)
        .map(|line| {
            line.trim_end_matches(" (*)")
                .trim_end_matches(" (proc-macro)")
        })
        .filter(|line| !line.is_empty())
        .collect();
    let workspace = packages.iter().filter(|line| is_path_source(line)).count();
    Closure {
        workspace,
        external: packages.len() - workspace,
    }
}

fn is_path_source(line: &str) -> bool {
    let Some(start) = line.find(" (") else {
        return false;
    };
    let source = &line[start + 2..];
    source.starts_with('/') || source.as_bytes().get(1) == Some(&b':')
}

/// Wall time of a clean release build into `target_dir`, then of a rebuild
/// after touching the binary's entry point (`src_path`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildTimes {
    pub clean: Duration,
    pub incremental: Duration,
}

/// Builds `product` clean into `target_dir`, then again after touching
/// `src_path`, and returns both wall times.
pub fn build(product: &Product, target_dir: &Path, src_path: &Path) -> Result<BuildTimes, String> {
    if target_dir.exists() {
        std::fs::remove_dir_all(target_dir)
            .map_err(|error| format!("clearing {}: {error}", target_dir.display()))?;
    }
    let clean = timed_build(product, target_dir)?;
    File::options()
        .append(true)
        .open(src_path)
        .and_then(|file| file.set_modified(SystemTime::now()))
        .map_err(|error| format!("touching {}: {error}", src_path.display()))?;
    let incremental = timed_build(product, target_dir)?;
    Ok(BuildTimes { clean, incremental })
}

fn timed_build(product: &Product, target_dir: &Path) -> Result<Duration, String> {
    let mut command = cargo();
    command.args(["build", "--release", "--locked", "-p", &product.package]);
    command.args(["--bin", &product.bin]);
    command.env("CARGO_TARGET_DIR", target_dir);
    with_features(&mut command, product);
    let started = Instant::now();
    run(&mut command)?;
    Ok(started.elapsed())
}

/// The built binary's path under `target_dir`.
pub fn binary(target_dir: &Path, bin: &str) -> PathBuf {
    target_dir
        .join("release")
        .join(format!("{bin}{}", std::env::consts::EXE_SUFFIX))
}

/// The entry-point source of each product's binary target, from
/// `cargo metadata`, in `products` order.
pub fn entry_points(products: &[Product]) -> Result<Vec<PathBuf>, String> {
    let mut command = cargo();
    command.args(["metadata", "--locked", "--no-deps", "--format-version", "1"]);
    let metadata = rusty_json::Value::parse(&run(&mut command)?)
        .map_err(|error| format!("parsing cargo metadata: {error}"))?;
    products
        .iter()
        .map(|product| entry_point(&metadata, product))
        .collect()
}

fn entry_point(metadata: &rusty_json::Value, product: &Product) -> Result<PathBuf, String> {
    let packages = metadata
        .get("packages")
        .and_then(rusty_json::Value::as_array)
        .ok_or("cargo metadata has no `packages`")?;
    let package = packages
        .iter()
        .find(|package| {
            package.get("name").and_then(rusty_json::Value::as_str) == Some(&product.package)
        })
        .ok_or_else(|| format!("no workspace package `{}`", product.package))?;
    let targets = package
        .get("targets")
        .and_then(rusty_json::Value::as_array)
        .ok_or_else(|| format!("`{}` has no `targets`", product.package))?;
    targets
        .iter()
        .filter(|target| is_bin(target))
        .find(|target| target.get("name").and_then(rusty_json::Value::as_str) == Some(&product.bin))
        .and_then(|target| target.get("src_path").and_then(rusty_json::Value::as_str))
        .map(PathBuf::from)
        .ok_or_else(|| format!("`{}` has no binary `{}`", product.package, product.bin))
}

fn is_bin(target: &rusty_json::Value) -> bool {
    target
        .get("kind")
        .and_then(rusty_json::Value::as_array)
        .is_some_and(|kinds| kinds.iter().any(|kind| kind.as_str() == Some("bin")))
}

fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

fn with_features(command: &mut Command, product: &Product) {
    if let Some(features) = &product.features {
        command.args(["--features", features]);
    }
}

/// Runs `command` to completion; its stdout on success, else its stderr's
/// last lines as the error.
fn run(command: &mut Command) -> Result<String, String> {
    let output = command
        .output()
        .map_err(|error| format!("running {command:?}: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(5).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(format!(
            "{:?} failed ({}): {}",
            command.get_program(),
            output.status,
            tail.join(" | ")
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| format!("cargo output is not UTF-8: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_distinct_workspace_and_external_packages_without_the_root() {
        let tree = "\
rush v0.1.0 (/w/crates/apps/rush)
rusty_libc v0.0.1 (/w/crates/foundation/rusty_libc)
serde v1.0.200
serde_derive v1.0.200 (proc-macro)
rusty_libc v0.0.1 (/w/crates/foundation/rusty_libc) (*)
serde v1.0.200 (*)
rusty_win32 v0.1.0 (C:\\w\\crates\\foundation\\rusty_win32)
forked v0.2.0 (https://github.com/x/forked#abc123)
";
        assert_eq!(
            count_closure(tree),
            Closure {
                workspace: 2,
                external: 3
            }
        );
    }

    #[test]
    fn a_package_with_no_dependencies_has_an_empty_closure() {
        assert_eq!(
            count_closure("leaf v0.1.0 (/w/leaf)\n"),
            Closure {
                workspace: 0,
                external: 0
            }
        );
    }
}

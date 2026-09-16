//! `mill-term` — MSYS2 & Git Bash replacement environment launcher for
//! Rusty Mill: hosts a real `rusty_term`-rendered terminal session running
//! `rush`, with a `rusty_git`-powered repo-status banner and the sibling
//! `rgit`/`rsed`/`rawk` tool binaries put on the child's `PATH`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;

use rusty_term::backend::Backend;
#[cfg(unix)]
use rusty_term::backend::UnixBackend;
#[cfg(windows)]
use rusty_term::backend::WindowsBackend;
use rusty_term::config::Config;
use rusty_term::core::Grid;

/// Finds a tool binary by walking up from this executable's own location
/// (robust to running as `target/{debug,release}/mill-term` under `cargo
/// run`, or as `target/{debug,release}/deps/mill_term-<hash>` under `cargo
/// test`). Checks each ancestor two ways: directly (every tool this
/// launches — `rush`, `rusty_git`, `rusty_text` — is a workspace sibling
/// now, see Cargo.toml, so their binaries share this executable's own
/// `target/{debug,release}/`), and under `repo/target/{debug,release}/
/// bin[.exe]`, kept as a fallback for a tool that's still a standalone
/// sibling checkout outside this workspace.
fn find_sibling_binary(repo: &str, bin: &str) -> Option<PathBuf> {
    let exe_name = if cfg!(windows) {
        format!("{bin}.exe")
    } else {
        bin.to_string()
    };
    let exe = std::env::current_exe().ok()?;
    for ancestor in exe.ancestors() {
        let candidate = ancestor.join(&exe_name);
        if candidate.is_file() {
            return Some(candidate);
        }
        for profile in ["debug", "release"] {
            let candidate = ancestor
                .join(repo)
                .join("target")
                .join(profile)
                .join(&exe_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Resolves `name` to run: prefer a real `PATH` install (the "properly
/// installed" case), falling back to a sibling dev-workspace build.
fn resolve_tool(repo: &str, name: &str) -> Option<PathBuf> {
    let exe_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(&exe_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    find_sibling_binary(repo, name)
}

/// Prints a real repo-status banner via `rusty_git` — a genuine
/// integration (branch + pending-change count), not a name-drop:
/// `rusty_git` was previously listed as a dependency here without a
/// single call into its API.
fn print_git_status(cwd: &Path) {
    match rusty_git::Repository::open(cwd) {
        Ok(repo) => {
            let branch = repo.current_branch();
            match repo.status() {
                Ok(entries) if entries.is_empty() => {
                    println!("Git branch: {branch} (clean)");
                }
                Ok(entries) => {
                    println!("Git branch: {branch} ({} pending change(s))", entries.len());
                }
                Err(e) => println!("Git branch: {branch} (status unavailable: {e})"),
            }
        }
        Err(_) => println!("Git: not a repository"),
    }
}

/// Prepends every sibling tool's directory to `PATH` so `rgit`/`rsed`/
/// `rawk` are actually reachable from inside the hosted rush session, not
/// just present somewhere on this machine's disk.
fn augmented_path(tools: &[PathBuf]) -> std::ffi::OsString {
    let mut dirs: Vec<PathBuf> = tools
        .iter()
        .filter_map(|p| p.parent().map(Path::to_path_buf))
        .collect();
    dirs.dedup();
    if let Some(existing) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(dirs).unwrap_or_default()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Rusty Mill Environment Launcher ===");
    println!("A real rusty_term session hosting rush, with rgit/rsed/rawk on PATH.\n");

    let current_dir = std::env::current_dir()?;
    let posix_cwd = rpath::win32_to_posix(&current_dir.to_string_lossy());
    println!("Workspace POSIX Path: {posix_cwd}");
    print_git_status(&current_dir);

    let Some(rush_path) = resolve_tool("rush", "rush") else {
        eprintln!("mill-term: couldn't find a `rush` binary on PATH or in a sibling `rush/target/{{debug,release}}`.");
        eprintln!("Build it first: (cd ../rush && cargo build).");
        std::process::exit(1);
    };

    let tool_paths: Vec<PathBuf> = [
        ("rusty_git", "rgit"),
        ("rusty_text", "rsed"),
        ("rusty_text", "rawk"),
    ]
    .iter()
    .filter_map(|(repo, bin)| resolve_tool(repo, bin))
    .collect();
    // SAFETY: single-threaded at this point in `main`, before the runtime
    // (matching rusty_term's own main.rs, which sets TERM/COLORTERM the
    // same way for the same reason).
    unsafe {
        std::env::set_var("PATH", augmented_path(&tool_paths));
    }
    if tool_paths.is_empty() {
        eprintln!(
            "mill-term: rgit/rsed/rawk not found (build rusty_git/rusty_text to enable them)"
        );
    }

    println!("Launching rush via rusty_term...\n");

    #[cfg(unix)]
    let backend: Box<dyn Backend> = Box::new(UnixBackend);
    #[cfg(windows)]
    let backend: Box<dyn Backend> = Box::new(WindowsBackend);

    let (init_cols, init_rows) = backend.terminal_size().unwrap_or((80, 24));
    let grid = Arc::new(Mutex::new(Grid::new(
        init_cols as usize,
        init_rows as usize,
    )));

    let config = Config {
        shell: Some(rush_path.to_string_lossy().into_owned()),
        cwd: Some(current_dir),
        ..Config::default()
    };

    let code = rusty_term::runtime::run(backend, grid, init_cols, init_rows, config)?;
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_real_sibling_binaries_built_earlier_this_session() {
        assert!(
            find_sibling_binary("rush", "rush").is_some(),
            "rush.exe should be found under ../rush/target/*"
        );
        assert!(find_sibling_binary("rusty_git", "rgit").is_some());
        assert!(find_sibling_binary("rusty_text", "rsed").is_some());
        assert!(find_sibling_binary("rusty_text", "rawk").is_some());
    }

    #[test]
    fn returns_none_for_a_nonexistent_sibling() {
        assert!(find_sibling_binary("no_such_repo_xyz", "no_such_bin_xyz").is_none());
    }

    #[test]
    fn augmented_path_prepends_tool_directories_and_keeps_existing_path() {
        // Forward slashes, not backslashes: `PathBuf::parent()` only
        // recognizes `\` as a separator on Windows, so a `C:\fake\...`
        // literal decomposes into a single opaque component on Unix and
        // this assertion failed on every `ubuntu-latest` run, every time
        // -- not a flake. `/` is accepted as a separator on both Windows
        // and Unix, so `.parent()` returns `/fake` on both.
        let tools = vec![
            PathBuf::from("/fake/rgit.exe"),
            PathBuf::from("/fake/rgit.exe"),
        ];
        let joined = augmented_path(&tools);
        let joined_str = joined.to_string_lossy();
        assert!(
            joined_str.starts_with("/fake"),
            "expected tool dir first, got: {joined_str}"
        );
        // The duplicate tool (same parent dir) must not appear twice.
        assert_eq!(joined_str.matches("/fake").count(), 1);
    }
}

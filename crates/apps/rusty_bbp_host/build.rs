//! Records the commit the binary was built from, so a run's output can say
//! which `bbp` produced it. `unknown` when git is unavailable; a `-dirty`
//! suffix when the tree had uncommitted changes. Run pins refuse both.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn main() {
    let sha = git(&["rev-parse", "HEAD"]).map_or_else(
        || "unknown".to_owned(),
        |head| match git(&["status", "--porcelain"]) {
            Some(s) if s.is_empty() => head,
            _ => format!("{head}-dirty"),
        },
    );
    println!("cargo:rustc-env=BBP_BUILD_SHA={sha}");
    // Rebuild when the commit, the branch it names, or any source changes: the
    // index alone misses an unstaged edit, and HEAD alone a commit on a branch.
    let mut watch = vec![
        "HEAD".to_owned(),
        "index".to_owned(),
        "packed-refs".to_owned(),
    ];
    watch.extend(git(&["symbolic-ref", "-q", "HEAD"]));
    for f in watch {
        if let Some(path) = git(&["rev-parse", "--git-path", &f]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(top) = git(&["rev-parse", "--show-toplevel"]) {
        for tracked in ["crates", "Cargo.toml", "Cargo.lock"] {
            println!("cargo:rerun-if-changed={top}/{tracked}");
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}

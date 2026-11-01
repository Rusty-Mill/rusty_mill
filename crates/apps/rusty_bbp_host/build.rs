//! Records the commit the binary was built from, so a run's output can say
//! which `bbp` produced it. `unknown` when git is unavailable; a `-dirty`
//! suffix when the tree had uncommitted changes (edited, staged or untracked
//! files, in this crate or any other in the checkout). Run pins refuse both.
//!
//! The commit is recomputed on every build. Cargo reruns a build script only
//! when a file it named has changed, and no file names "the working tree is
//! dirty": git's HEAD file does not change when a commit moves a branch, the
//! index does not change when a file is edited without `git add`, and a
//! dependency's source is outside this package. Any such list leaves a binary
//! built from edited code reporting the clean commit it was last built at. So
//! the script names a path that never exists, which cargo treats as always
//! changed, and asks git each time. A rebuild whose answer is the same costs
//! one `git status`; a different answer recompiles the crate with the new sha.
//!
//! `--untracked-files=normal` overrides a user's `status.showUntrackedFiles=no`,
//! which would otherwise hide an untracked source file and report clean (#1328).

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        // Reading the state must not rewrite the index (that would touch the
        // tree it is describing).
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn main() {
    let sha = git(&["rev-parse", "HEAD"]).map_or_else(
        || "unknown".to_owned(),
        |head| match git(&["status", "--porcelain", "--untracked-files=normal"]) {
            Some(s) if s.is_empty() => head,
            _ => format!("{head}-dirty"),
        },
    );
    println!("cargo:rustc-env=BBP_BUILD_SHA={sha}");
    println!("cargo:rerun-if-changed=.bbp-build-sha-is-recomputed-every-build");
}

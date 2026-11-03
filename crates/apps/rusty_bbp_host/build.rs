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
//! four quick git calls; a different answer recompiles the crate with the new sha.
//!
//! The tree check is `src/clean_tree.rs`, shared with the `bbp pairs` pin: a
//! plain `git status` misses changes hidden by git settings (#1328, #1361).

use std::path::Path;

#[allow(dead_code)] // the pin uses parts this script does not
#[path = "src/clean_tree.rs"]
mod clean_tree;

/// The build's commit, `-dirty` when the tree is not exactly it, or `None`
/// when git cannot say which commit this is.
fn build_sha(dir: &Path) -> Option<String> {
    let head = clean_tree::git(dir, &["rev-parse", "HEAD"]).ok()?;
    if !head.ok {
        return None;
    }
    Some(match clean_tree::dirt(dir) {
        Ok(None) => head.line().to_owned(),
        _ => format!("{}-dirty", head.line()),
    })
}

fn main() {
    let sha = std::env::var("CARGO_MANIFEST_DIR")
        .ok()
        .and_then(|dir| build_sha(Path::new(&dir)))
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=BBP_BUILD_SHA={sha}");
    println!("cargo:rerun-if-changed=.bbp-build-sha-is-recomputed-every-build");
}

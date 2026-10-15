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
    // Rebuild when the checked-out commit changes.
    if let Some(dir) = git(&["rev-parse", "--git-dir"]) {
        println!("cargo:rerun-if-changed={dir}/HEAD");
        println!("cargo:rerun-if-changed={dir}/index");
    }
    println!("cargo:rerun-if-changed=build.rs");
}

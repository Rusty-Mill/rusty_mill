//! Dry runs over real data (#598 gap 2), `bbp dry-run`.
//!
//! A change that would write real data is never run for real by an agent:
//! that is an irreversible claim, which only Nano accepts. What runs is a
//! dry run: the program, confined with no network or socket endpoints, may
//! read the named `real_data` roots and the system roots, and may write
//! only its own plan directory. It writes the plan of what it would change
//! to `$BBP_PLAN`, one JSON object per line.
//!
//! The plan is `sensitive` and stays on the host, in the plan directory.
//! Only its digest and entry count, which are `internal`, are reported, for
//! the claim to cite.

use rusty_bbp::Sha256;
use rusty_sandbox::{Executor, Limits, SandboxSpec, Termination};
use rusty_serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The env var naming the plan file the program writes.
pub const PLAN_ENV: &str = "BBP_PLAN";

/// System roots a dry run may read, when they exist.
const SYSTEM_ROOTS: [&str; 5] = ["/bin", "/usr", "/lib", "/lib64", "/etc/alternatives"];

/// One dry run.
#[derive(Clone, Debug)]
pub struct Request {
    /// `[a-z0-9_-]+`, naming the plan directory.
    pub name: String,
    /// The `real_data` roots, as `(name, absolute path)`.
    pub data: Vec<(String, PathBuf)>,
    /// The plan directory's parent, outside every data root.
    pub work: PathBuf,
    pub program: String,
    pub args: Vec<String>,
    pub wall: Duration,
}

/// What is reported of a finished dry run: its counts, never its plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub name: String,
    /// The plan file, on the host.
    pub plan: String,
    pub sha256: Sha256,
    /// Non-empty lines in the plan.
    pub entries: u64,
}

/// Run `req` under `exec` and summarise its plan.
pub fn run<E: Executor>(exec: &E, req: &Request) -> Result<Summary, String>
where
    E::Error: std::fmt::Display,
{
    check(req)?;
    let dir =
        std::path::absolute(req.work.join("dry-run").join(&req.name)).map_err(|e| e.to_string())?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let plan = dir.join("plan.jsonl");
    let spec = spec(req, &dir, &plan)?;
    let out = exec
        .exec(&spec, &req.program, &req.args)
        .map_err(|e| format!("dry run {}: {e}", req.name))?;
    if out.termination != Termination::Exited(0) {
        return Err(format!(
            "dry run {} ended {:?}: {}",
            req.name,
            out.termination,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let bytes = std::fs::read(&plan).map_err(|e| format!("{}: {e}", plan.display()))?;
    Ok(Summary {
        name: req.name.clone(),
        plan: plan.to_string_lossy().into_owned(),
        sha256: Sha256::of(&bytes),
        entries: entries(&bytes),
    })
}

/// Refuse a bad name, a relative or missing data root, and a work
/// directory that lies in a data root or holds one.
fn check(req: &Request) -> Result<(), String> {
    let ok = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    };
    if !ok(&req.name) {
        return Err(format!("dry run name {:?}: [a-z0-9_-]+ required", req.name));
    }
    if req.data.is_empty() {
        return Err("a dry run needs at least one real_data root".into());
    }
    let work = std::path::absolute(&req.work).map_err(|e| e.to_string())?;
    for (name, path) in &req.data {
        if !ok(name) {
            return Err(format!("real_data name {name:?}: [a-z0-9_-]+ required"));
        }
        if !path.is_absolute() || !path.is_dir() {
            return Err(format!(
                "real_data {name}: {} is not an absolute directory",
                path.display()
            ));
        }
        if work.starts_with(path) || path.starts_with(&work) {
            return Err(format!(
                "real_data {name}: {} overlaps the work directory {}",
                path.display(),
                work.display()
            ));
        }
    }
    Ok(())
}

/// Read the data and system roots (and the program's directory), write only
/// `dir`, with `$BBP_PLAN` and one `$BBP_DATA_<NAME>` per data root.
fn spec(req: &Request, dir: &Path, plan: &Path) -> Result<SandboxSpec, String> {
    let mut read: Vec<PathBuf> = SYSTEM_ROOTS
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect();
    read.extend(req.data.iter().map(|(_, p)| p.clone()));
    if let Some(parent) = Path::new(&req.program).parent().filter(|p| p.is_absolute()) {
        read.push(parent.to_path_buf());
    }
    let mut env = vec![
        ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
        ("HOME".to_owned(), dir.to_string_lossy().into_owned()),
        (PLAN_ENV.to_owned(), plan.to_string_lossy().into_owned()),
    ];
    env.extend(req.data.iter().map(|(name, p)| {
        (
            format!("BBP_DATA_{}", name.to_ascii_uppercase().replace('-', "_")),
            p.to_string_lossy().into_owned(),
        )
    }));
    let wall = req
        .wall
        .clamp(Duration::from_secs(1), Duration::from_secs(24 * 3600));
    let limits =
        Limits::new(wall, wall, 4 << 30, 1 << 30, 1_024, 512).map_err(|e| e.to_string())?;
    SandboxSpec::new(
        read,
        vec![dir.to_path_buf(), PathBuf::from("/dev/null")],
        dir.to_path_buf(),
        env,
        limits,
    )
    .map_err(|e| format!("dry run sandbox: {e}"))
}

/// Non-empty lines of a plan.
fn entries(bytes: &[u8]) -> u64 {
    bytes
        .split(|b| *b == b'\n')
        .filter(|l| !l.iter().all(u8::is_ascii_whitespace))
        .count() as u64
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::runner::Unconfined;

    fn root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("bbp_dry_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("data")).unwrap();
        root
    }

    fn request(root: &Path, script: &str) -> Request {
        Request {
            name: "rename".into(),
            data: vec![("crm".into(), root.join("data"))],
            work: root.join("work"),
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            wall: Duration::from_secs(10),
        }
    }

    #[test]
    fn a_dry_run_reports_its_plans_digest_and_count_only() {
        let root = root("ok");
        std::fs::write(root.join("data/a.csv"), "x\ny\n").unwrap();
        let req = request(
            &root,
            "wc -l < \"$BBP_DATA_CRM/a.csv\" >/dev/null && printf '{\"op\":1}\\n\\n{\"op\":2}\\n' > \"$BBP_PLAN\"",
        );
        let s = run(&Unconfined, &req).unwrap();
        assert_eq!(s.entries, 2);
        let plan = std::fs::read(&s.plan).unwrap();
        assert_eq!(s.sha256, Sha256::of(&plan));
        assert!(s.plan.starts_with(root.join("work").to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_or_badly_placed_dry_run_is_refused() {
        let root = root("bad");
        assert!(run(&Unconfined, &request(&root, "exit 3"))
            .unwrap_err()
            .contains("Exited(3)"));
        let mut req = request(&root, "true");
        req.work = root.join("data/work");
        assert!(run(&Unconfined, &req).unwrap_err().contains("overlaps"));
        req = request(&root, "true");
        req.name = "../x".into();
        assert!(run(&Unconfined, &req).is_err());
        req = request(&root, "true");
        req.data = vec![("crm".into(), PathBuf::from("relative"))];
        assert!(run(&Unconfined, &req).unwrap_err().contains("absolute"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

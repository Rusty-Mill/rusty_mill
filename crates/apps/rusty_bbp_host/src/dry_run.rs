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
//! the claim to cite. So is what the program prints: a failed run reports
//! how it ended, and its output stays in the plan directory.
//!
//! Every path is resolved through symlinks before anything is removed,
//! made or granted, and the resolved paths are the ones used: a work
//! directory that is, holds or lies in a data root is refused, whatever
//! names lead there.

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
    let req = &checked(req)?;
    let dir = req.work.join("dry-run").join(&req.name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let plan = dir.join("plan.jsonl");
    let spec = spec(req, &dir, &plan)?;
    let out = exec
        .exec(&spec, &req.program, &req.args)
        .map_err(|e| format!("dry run {}: {e}", req.name))?;
    // What it printed may be real data: kept beside the plan, never returned.
    for (name, bytes) in [("stdout.log", &out.stdout), ("stderr.log", &out.stderr)] {
        let path = dir.join(name);
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    if out.termination != Termination::Exited(0) {
        return Err(format!(
            "dry run {} ended {}; its output is in {}",
            req.name,
            ended(&out.termination),
            dir.display()
        ));
    }
    let meta = std::fs::symlink_metadata(&plan).map_err(|e| format!("{}: {e}", plan.display()))?;
    if !meta.is_file() {
        return Err(format!(
            "{}: the plan is not a regular file",
            plan.display()
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

/// The sandbox executor's state directory for work directory `work`,
/// resolved like the paths the run is granted.
pub fn state_dir(work: &Path) -> Result<PathBuf, String> {
    let work = std::path::absolute(work).map_err(|e| e.to_string())?;
    Ok(resolved(&work)?.join("dry-run.state"))
}

/// How a run ended, without anything it printed.
fn ended(t: &Termination) -> String {
    match t {
        Termination::Exited(code) => format!("with exit code {code}"),
        other => format!("{other:?}"),
    }
}

/// `req` with its data roots and work directory resolved through symlinks.
/// Refuses a bad name, a relative or missing data root, and a work
/// directory, or plan directory, that is, lies in or holds a data root.
/// Creates and removes nothing.
fn checked(req: &Request) -> Result<Request, String> {
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
    let work = resolved(&work)?;
    let dir = resolved(&work.join("dry-run").join(&req.name))?;
    let mut data = Vec::with_capacity(req.data.len());
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
        let real = path
            .canonicalize()
            .map_err(|e| format!("real_data {name}: {}: {e}", path.display()))?;
        let overlaps = |p: &Path| p.starts_with(&real) || real.starts_with(p);
        if overlaps(&work) || overlaps(&dir) {
            return Err(format!(
                "real_data {name}: {} overlaps the work directory {}",
                real.display(),
                work.display()
            ));
        }
        data.push((name.clone(), real));
    }
    if dir != work.join("dry-run").join(&req.name) {
        return Err(format!(
            "dry run {}: its plan directory {} is reached through a link",
            req.name,
            dir.display()
        ));
    }
    Ok(Request {
        data,
        work,
        ..req.clone()
    })
}

/// `path` with its existing ancestors resolved through symlinks; the rest
/// must be plain names.
fn resolved(path: &Path) -> Result<PathBuf, String> {
    if path.symlink_metadata().is_ok() {
        return path
            .canonicalize()
            .map_err(|e| format!("{}: {e}", path.display()));
    }
    let (Some(parent), Some(std::path::Component::Normal(name))) =
        (path.parent(), path.components().next_back())
    else {
        return Err(format!("{}: not a normal path", path.display()));
    };
    Ok(resolved(parent)?.join(name))
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
            .contains("exit code 3"));
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

    /// An executor that ends every run `how`, having printed `secret`.
    struct Ends(Termination, &'static str);

    impl Executor for Ends {
        type Error = String;
        fn exec(
            &self,
            _: &SandboxSpec,
            _: &str,
            _: &[String],
        ) -> Result<rusty_sandbox::ExecOutcome, String> {
            Ok(rusty_sandbox::ExecOutcome {
                termination: self.0,
                stdout: self.1.as_bytes().to_vec(),
                stderr: self.1.as_bytes().to_vec(),
                wall: Duration::ZERO,
            })
        }
    }

    #[test]
    fn a_failed_dry_run_never_returns_what_it_printed() {
        const SECRET: &str = "ssn=078-05-1120";
        let root = root("quiet");
        std::fs::write(root.join("data/a.csv"), SECRET).unwrap();
        let script = "cat \"$BBP_DATA_CRM/a.csv\"; cat \"$BBP_DATA_CRM/a.csv\" >&2; exit 4";
        let err = run(&Unconfined, &request(&root, script)).unwrap_err();
        assert!(!err.contains(SECRET), "{err}");
        assert!(err.contains("exit code 4"), "{err}");
        let kept = root.join("work/dry-run/rename/stderr.log");
        assert_eq!(
            std::fs::read_to_string(kept).unwrap(),
            SECRET,
            "kept locally"
        );
        for how in [Termination::TimedOut, Termination::Signaled(9)] {
            let err = run(&Ends(how, SECRET), &request(&root, "true")).unwrap_err();
            assert!(!err.contains(SECRET), "{err}");
            assert!(err.contains(&format!("{how:?}")), "{err}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `root/data/keep` exists before every refused run and after it.
    fn sentinel(root: &Path) -> PathBuf {
        let keep = root.join("data/keep");
        std::fs::write(&keep, "real").unwrap();
        keep
    }

    #[cfg(unix)]
    #[test]
    fn a_work_directory_reached_through_a_link_into_the_data_is_refused() {
        use std::os::unix::fs::symlink;
        let root = root("links");
        let keep = sentinel(&root);
        // A plan directory already in the data, which a lexical check missed
        // and the old code removed before anything was refused.
        let planted = root.join("data/dry-run/rename");
        std::fs::create_dir_all(&planted).unwrap();
        std::fs::write(planted.join("row"), "real").unwrap();
        symlink(root.join("data"), root.join("data-alias")).unwrap();
        symlink(root.join("data"), root.join("work-link")).unwrap();
        std::fs::create_dir_all(root.join("outer")).unwrap();
        symlink(root.join("data"), root.join("outer/mid")).unwrap();
        let cases: [(&str, PathBuf, PathBuf); 3] = [
            (
                "aliased data",
                root.join("data-alias"),
                root.join("data/work"),
            ),
            ("linked work", root.join("data"), root.join("work-link")),
            (
                "linked ancestor",
                root.join("data"),
                root.join("outer/mid/w"),
            ),
        ];
        for (case, data, work) in cases {
            let mut req = request(&root, "echo x > \"$BBP_DATA_CRM/keep\"");
            req.data = vec![("crm".into(), data)];
            req.work = work;
            let err = run(&Unconfined, &req).unwrap_err();
            assert!(err.contains("overlaps"), "{case}: {err}");
            assert_eq!(std::fs::read_to_string(&keep).unwrap(), "real", "{case}");
            assert!(planted.join("row").exists(), "{case}: nothing removed");
        }
        // A plan directory linked out of the work directory is refused too.
        std::fs::create_dir_all(root.join("work/dry-run")).unwrap();
        std::fs::create_dir_all(root.join("elsewhere")).unwrap();
        symlink(root.join("elsewhere"), root.join("work/dry-run/rename")).unwrap();
        let err = run(&Unconfined, &request(&root, "true")).unwrap_err();
        assert!(err.contains("through a link"), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn the_granted_paths_are_the_resolved_ones() {
        use std::os::unix::fs::symlink;
        let root = root("resolved");
        symlink(root.join("data"), root.join("data-alias")).unwrap();
        let mut req = request(&root, "true");
        req.data = vec![("crm".into(), root.join("data-alias"))];
        let real = checked(&req).unwrap();
        assert_eq!(real.data[0].1, root.join("data").canonicalize().unwrap());
        assert_eq!(
            state_dir(&root.join("work")).unwrap(),
            root.canonicalize().unwrap().join("work/dry-run.state")
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

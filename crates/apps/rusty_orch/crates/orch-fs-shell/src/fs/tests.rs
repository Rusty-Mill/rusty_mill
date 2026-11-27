use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

/// A fresh directory under the system temp dir, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("orch-fs-shell-{}-{n}", std::process::id()));
        let _stale = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("root/src")).expect("scratch dir");
        Scratch(dir)
    }

    fn root(&self) -> PathBuf {
        self.0.join("root")
    }

    fn ws(&self) -> FsWorkspace {
        FsWorkspace::new(self.root()).expect("workspace")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _best_effort = fs::remove_dir_all(&self.0);
    }
}

fn p(raw: &str) -> SessionPath {
    SessionPath::parse(raw).expect("session path")
}

fn spec(argv: &[&str], timeout: Duration) -> ExecSpec {
    ExecSpec {
        argv: argv.iter().map(|a| (*a).to_owned()).collect(),
        cwd: SessionPath::root(),
        timeout,
        stdin: Vec::new(),
    }
}

#[test]
fn a_root_that_is_not_a_directory_is_refused() {
    let s = Scratch::new();
    fs::write(s.0.join("file"), "x").expect("file");
    assert!(FsWorkspace::new(s.0.join("file")).is_err());
    assert!(FsWorkspace::new(s.0.join("missing")).is_err());
}

#[test]
fn write_read_list_stat_round_trip() {
    let s = Scratch::new();
    let ws = s.ws();
    ws.write(&p("src/lib.rs"), b"fn main() {}").expect("write");
    assert_eq!(
        ws.read(&p("src/lib.rs"), 1024).expect("read"),
        (b"fn main() {}".to_vec(), false)
    );
    assert_eq!(
        ws.read(&p("src/lib.rs"), 2).expect("read"),
        (b"fn".to_vec(), true)
    );

    let listed = ws.list(&SessionPath::root()).expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (listed[0].name.as_str(), listed[0].kind),
        ("src", Kind::Dir)
    );

    let entry = ws.stat(&p("src/lib.rs")).expect("stat");
    assert_eq!((entry.kind, entry.size), (Kind::File, 12));
    assert!(ws.write(&p("nodir/x"), b"x").is_err(), "parent must exist");
}

#[cfg(unix)]
#[test]
fn symlinks_out_of_the_root_are_refused() {
    use std::os::unix::fs::symlink;

    let s = Scratch::new();
    fs::write(s.0.join("secret"), "s3cret").expect("outside file");
    let root = s.root();
    symlink("/proc", root.join("proc")).expect("link");
    symlink("/etc/passwd", root.join("passwd")).expect("link");
    symlink(s.0.join("secret"), root.join("secret")).expect("link");
    symlink(&s.0, root.join("up")).expect("link");
    symlink(s.0.join("new"), root.join("dangling")).expect("link");
    symlink("src", root.join("inner")).expect("link");
    let ws = s.ws();

    for path in ["proc/self/environ", "passwd", "secret", "up/secret"] {
        let err = ws.read(&p(path), 1024).expect_err(path);
        assert!(err.contains("outside the session"), "{path}: {err}");
    }
    assert!(ws.list(&p("proc")).is_err());
    assert!(ws.stat(&p("up")).is_err());
    assert!(ws.write(&p("secret"), b"x").is_err());
    assert!(ws.write(&p("up/planted"), b"x").is_err());
    assert!(ws.write(&p("dangling"), b"x").is_err());
    assert!(!s.0.join("planted").exists() && !s.0.join("new").exists());
    assert_eq!(
        fs::read_to_string(s.0.join("secret")).expect("read"),
        "s3cret"
    );

    let mut cwd_out = spec(&["pwd"], Duration::from_secs(5));
    cwd_out.cwd = p("up");
    assert!(ws.exec(&cwd_out).is_err());

    // A link that stays inside is followed; `list` reports it unfollowed.
    ws.write(&p("inner/a"), b"in")
        .expect("write via inner link");
    assert_eq!(ws.read(&p("src/a"), 8).expect("read").0, b"in");
    let kinds: Vec<_> = ws
        .list(&SessionPath::root())
        .expect("list")
        .into_iter()
        .map(|e| (e.name, e.kind))
        .collect();
    assert!(kinds.contains(&("inner".to_owned(), Kind::Symlink)));
}

#[cfg(unix)]
#[test]
fn exec_runs_in_the_root_with_stdin() {
    let s = Scratch::new();
    let ws = s.ws();
    let out = ws
        .exec(&spec(&["pwd"], Duration::from_secs(5)))
        .expect("pwd");
    let root = s.root().canonicalize().expect("canonical");
    assert_eq!(out.stdout, format!("{}\n", root.display()).into_bytes());
    assert_eq!((out.status, out.timed_out), (Some(0), false));

    let mut cat = spec(&["cat"], Duration::from_secs(5));
    cat.stdin = b"piped".to_vec();
    cat.cwd = p("src");
    assert_eq!(ws.exec(&cat).expect("cat").stdout, b"piped");

    let fail = ws
        .exec(&spec(
            &["sh", "-c", "echo e >&2; exit 3"],
            Duration::from_secs(5),
        ))
        .expect("sh");
    assert_eq!(
        (fail.status, fail.stderr.as_slice()),
        (Some(3), &b"e\n"[..])
    );

    let err = ws
        .exec(&spec(&["no-such-program-xyz"], Duration::from_secs(5)))
        .expect_err("missing program");
    assert!(err.contains("could not start"));
}

#[cfg(unix)]
#[test]
fn exec_is_killed_at_its_timeout() {
    let s = Scratch::new();
    let started = Instant::now();
    let out = s
        .ws()
        .exec(&spec(&["sleep", "30"], Duration::from_millis(200)))
        .expect("sleep");
    assert!(out.timed_out);
    assert_eq!(out.status, None);
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[cfg(unix)]
#[test]
fn exec_output_is_capped() {
    let s = Scratch::new();
    let script = format!("head -c {} /dev/zero", OUTPUT_CAP + 4096);
    let out = s
        .ws()
        .exec(&spec(&["sh", "-c", &script], Duration::from_secs(30)))
        .expect("head");
    assert_eq!(out.stdout.len(), OUTPUT_CAP);
    assert!(out.truncated);
    assert_eq!(
        out.status,
        Some(0),
        "the child was not blocked on a full pipe"
    );
}

//! Real git, real sandbox: a card's worktree, a confined command, a runner-made commit.
//!
//! These tests exercise Landlock and seccomp, so they run on Linux only. Where
//! the kernel cannot confine they FAIL rather than skip: a green run must mean the
//! confinement was exercised. Set `MILL_ALLOW_NO_SANDBOX=1` to skip them knowingly.

#![cfg(target_os = "linux")]

use std::io::ErrorKind;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram, UnixListener};
use std::path::{Path, PathBuf};
use std::process::Command;

use rusty_mill_board::domain::Card;
use rusty_mill_runner::config::{Stage, parse};
use rusty_mill_runner::git_work::GitWork;
use rusty_mill_runner::ports::{Outcome, Work};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

struct Fixture {
    root: PathBuf,
    work: GitWork,
    card: Card,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn fixture(name: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("mill_runner_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).expect("repo dir");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@t"]);
    git(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("README"), "hi\n").expect("write");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let helper = PathBuf::from(env!("CARGO_BIN_EXE_mill-runner"));
    let work = GitWork::new(&repo, &root.join("work"), "main", helper, vec![]).expect("work");
    let card = Card::new("Add file", "body", vec![], 0).expect("card");
    Fixture { root, work, card }
}

fn stage(network: bool) -> Stage {
    let json = format!(
        r#"[{{"name":"build","from":["ready"],"role":"builder","argv":["x"],"wall_secs":20,"network":{network}}}]"#
    );
    parse(&json).expect("stage").remove(0)
}

fn skip_allowed() -> bool {
    std::env::var_os("MILL_ALLOW_NO_SANDBOX").is_some()
}

/// Run `argv` confined in the card's worktree. `None` only when the operator
/// explicitly allowed an unavailable sandbox; otherwise a failure to confine is a test failure.
fn run_argv(f: &Fixture, s: &Stage, argv: &[&str]) -> Option<Outcome> {
    let dir = f.work.prepare(&f.card).expect("prepare");
    let argv: Vec<String> = argv.iter().map(|a| (*a).to_owned()).collect();
    match f
        .work
        .run(s, &argv, &dir, &[("PATH".into(), "/usr/bin:/bin".into())])
    {
        Ok(o) => Some(o),
        Err(e) if skip_allowed() => {
            eprintln!("SKIPPED (MILL_ALLOW_NO_SANDBOX set): sandbox unavailable here: {e}");
            None
        }
        Err(e) => panic!("the sandbox could not confine the command, so nothing was tested: {e}"),
    }
}

fn sh(f: &Fixture, s: &Stage, script: &str) -> Option<Outcome> {
    run_argv(f, s, &["/bin/sh", "-c", script])
}

#[test]
fn command_edits_the_worktree_and_the_runner_commits_it_on_the_card_branch() {
    let f = fixture("commit");
    let Some(out) = sh(&f, &stage(false), "echo built > out.txt && echo done") else {
        return;
    };
    assert!(out.ok, "{out:?}");
    assert!(out.tail.contains("done"));

    let dir = f.work.prepare(&f.card).expect("dir");
    assert!(
        f.work
            .commit(&dir, "mill: Add file [build]")
            .expect("commit")
    );
    assert!(
        !f.work.commit(&dir, "again").expect("commit"),
        "nothing new to commit"
    );

    let repo = f.root.join("repo");
    let branch = format!("mill/{}", &f.card.id.to_string()[..8]);
    assert_eq!(
        git(&repo, &["log", "-1", "--format=%s", &branch]),
        "mill: Add file [build]"
    );
    assert_eq!(git(&repo, &["show", &format!("{branch}:out.txt")]), "built");
    assert!(
        !git(&repo, &["ls-tree", "-r", "--name-only", "main"]).contains("out.txt"),
        "main is untouched"
    );
}

#[test]
fn command_cannot_write_outside_its_worktree_or_into_git() {
    let f = fixture("escape");
    let outside = f.root.join("escaped.txt");
    let script = format!(
        "echo x > {}; echo y > {}/repo/.git/pwned; echo z > ../sibling.txt",
        outside.display(),
        f.root.display()
    );
    let Some(out) = sh(&f, &stage(false), &script) else {
        return;
    };
    assert!(!out.ok, "writes outside the sandbox must fail: {out:?}");
    assert!(!outside.exists());
    assert!(!f.root.join("repo/.git/pwned").exists());
    assert!(!f.root.join("work/sibling.txt").exists());
}

#[test]
fn failure_and_timeout_are_outcomes_not_errors() {
    let f = fixture("outcomes");
    let Some(bad) = sh(&f, &stage(false), "echo boom >&2; exit 3") else {
        return;
    };
    assert!(
        !bad.ok && bad.summary.starts_with("exit 3") && bad.tail.contains("boom"),
        "{bad:?}"
    );
    let mut slow = stage(false);
    slow.wall_secs = 1;
    let Some(timed) = sh(&f, &slow, "sleep 30") else {
        return;
    };
    assert!(
        !timed.ok && timed.summary.contains("timed out"),
        "{timed:?}"
    );
}

fn require_python() {
    let have = Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(
        have,
        "python3 is required for the socket tests (the CI image has it)"
    );
}

/// A script that tries to reach `target` over each local-socket kind and prints what happened.
const TRY_LOCAL: &str = r#"
import socket, sys
path, abstract = sys.argv[1], sys.argv[2]
for name, family, addr in (("pathname", socket.AF_UNIX, path), ("abstract", socket.AF_UNIX, "\0" + abstract)):
    try:
        s = socket.socket(family, socket.SOCK_STREAM)
        s.connect(addr)
        print(name, "CONNECTED")
    except PermissionError:
        print(name, "denied")
    except OSError as e:
        print(name, "oserror", e.errno)
"#;

/// Listeners outside the sandbox that stand for a local control daemon.
struct Daemon {
    path: UnixListener,
    abstract_: UnixListener,
    abstract_name: String,
    socket_path: PathBuf,
}

fn daemon(f: &Fixture) -> Daemon {
    let socket_path = f.root.join("daemon.sock");
    let path = UnixListener::bind(&socket_path).expect("bind path socket");
    let unique = f
        .root
        .file_name()
        .expect("name")
        .to_string_lossy()
        .into_owned();
    let abstract_name = format!("mill-test-{unique}");
    let addr = SocketAddr::from_abstract_name(abstract_name.as_bytes()).expect("abstract addr");
    let abstract_ = UnixListener::bind_addr(&addr).expect("bind abstract socket");
    for l in [&path, &abstract_] {
        l.set_nonblocking(true).expect("nonblocking");
    }
    Daemon {
        path,
        abstract_,
        abstract_name,
        socket_path,
    }
}

fn nobody_connected(d: &Daemon) -> bool {
    [&d.path, &d.abstract_]
        .iter()
        .all(|l| matches!(l.accept(), Err(e) if e.kind() == ErrorKind::WouldBlock))
}

#[test]
fn a_command_without_network_reaches_no_local_daemon_over_any_socket_kind() {
    require_python();
    let f = fixture("noendpoints");
    let d = daemon(&f);
    let argv = [
        "/usr/bin/python3",
        "-c",
        TRY_LOCAL,
        d.socket_path.to_str().expect("utf8"),
        &d.abstract_name,
    ];
    let Some(out) = run_argv(&f, &stage(false), &argv) else {
        return;
    };
    assert!(out.ok, "{out:?}");
    assert!(
        out.tail.contains("pathname denied") && out.tail.contains("abstract denied"),
        "{}",
        out.tail
    );
    assert!(!out.tail.contains("CONNECTED"));
    assert!(
        nobody_connected(&d),
        "the daemon must never see a connection"
    );
}

#[test]
fn a_command_with_network_reaches_the_internet_family_but_no_local_daemon() {
    require_python();
    let f = fixture("internetonly");
    let d = daemon(&f);
    let tcp = std::net::TcpListener::bind("127.0.0.1:0").expect("tcp");
    tcp.set_nonblocking(true).expect("nonblocking");
    let port = tcp.local_addr().expect("addr").port().to_string();

    // Local sockets are denied exactly as without network...
    let argv = [
        "/usr/bin/python3",
        "-c",
        TRY_LOCAL,
        d.socket_path.to_str().expect("utf8"),
        &d.abstract_name,
    ];
    let Some(out) = run_argv(&f, &stage(true), &argv) else {
        return;
    };
    assert!(
        out.tail.contains("pathname denied") && out.tail.contains("abstract denied"),
        "{}",
        out.tail
    );
    assert!(
        nobody_connected(&d),
        "the daemon must never see a connection"
    );

    // ...while an IP connection (what an agent CLI needs to reach its model) works.
    let code = "import socket,sys; s=socket.create_connection(('127.0.0.1', int(sys.argv[1])), timeout=5); print('tcp ok')";
    let Some(tcp_out) = run_argv(&f, &stage(true), &["/usr/bin/python3", "-c", code, &port]) else {
        return;
    };
    assert!(tcp_out.ok && tcp_out.tail.contains("tcp ok"), "{tcp_out:?}");
    let accepted = (0..50).any(|_| {
        std::thread::sleep(std::time::Duration::from_millis(40));
        tcp.accept().is_ok()
    });
    assert!(accepted, "the TCP listener saw the connection");

    // ...and a stage without network cannot make that TCP connection.
    let Some(blocked) = run_argv(&f, &stage(false), &["/usr/bin/python3", "-c", code, &port])
    else {
        return;
    };
    assert!(!blocked.ok, "no network, no TCP: {blocked:?}");
}

/// Tries to turn a `socketpair` into a way out: a datagram pair re-pointed at an outside endpoint.
const TRY_PAIRS: &str = r#"
import socket, sys
path, abstract, stream_abstract = sys.argv[1], sys.argv[2], sys.argv[3]
for name, addr in (("pathname", path), ("abstract", "\0" + abstract)):
    try:
        a, b = socket.socketpair(socket.AF_UNIX, socket.SOCK_DGRAM)
    except PermissionError:
        print(name, "datagram pair denied")
        continue
    a.connect(addr)
    a.send(b"escaped")
    print(name, "SENT")
# Stream pairs stay available (a linker is started through one) but cannot be re-pointed.
a, b = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
a.send(b"ping")
print("stream pair works:", b.recv(4) == b"ping")
try:
    a.connect("\0" + stream_abstract)
    print("stream pair CONNECTED")
except OSError as e:
    print("stream pair reconnect refused", e.errno)
"#;

#[test]
fn a_datagram_socketpair_cannot_be_pointed_at_an_outside_endpoint() {
    require_python();
    for network in [false, true] {
        let f = fixture(&format!("pairs{network}"));
        let unique = f
            .root
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        let (abstract_name, stream_name) =
            (format!("mill-dg-{unique}"), format!("mill-st-{unique}"));
        let path = f.root.join("dgram.sock");

        // Datagram and stream endpoints outside the sandbox, standing for local daemons.
        let dg_path = UnixDatagram::bind(&path).expect("bind datagram path");
        let dg_abs = UnixDatagram::bind_addr(
            &SocketAddr::from_abstract_name(abstract_name.as_bytes()).expect("addr"),
        )
        .expect("bind datagram abstract");
        let st_abs = UnixListener::bind_addr(
            &SocketAddr::from_abstract_name(stream_name.as_bytes()).expect("addr"),
        )
        .expect("bind stream abstract");
        for d in [&dg_path, &dg_abs] {
            d.set_nonblocking(true).expect("nonblocking");
        }
        st_abs.set_nonblocking(true).expect("nonblocking");

        let argv = [
            "/usr/bin/python3",
            "-c",
            TRY_PAIRS,
            path.to_str().expect("utf8"),
            &abstract_name,
            &stream_name,
        ];
        let Some(out) = run_argv(&f, &stage(network), &argv) else {
            return;
        };
        let what = format!("network={network}: {}", out.tail);
        assert!(out.ok, "{what}");
        assert!(out.tail.contains("pathname datagram pair denied"), "{what}");
        assert!(out.tail.contains("abstract datagram pair denied"), "{what}");
        assert!(!out.tail.contains("SENT"), "{what}");
        assert!(
            out.tail.contains("stream pair works: True"),
            "stream pairs must keep working: {what}"
        );
        assert!(out.tail.contains("stream pair reconnect refused"), "{what}");
        assert!(!out.tail.contains("CONNECTED"), "{what}");

        let mut buf = [0u8; 16];
        for (name, d) in [("pathname", &dg_path), ("abstract", &dg_abs)] {
            assert!(
                matches!(d.recv(&mut buf), Err(e) if e.kind() == ErrorKind::WouldBlock),
                "the {name} datagram endpoint must receive nothing (network={network})"
            );
        }
        assert!(
            matches!(st_abs.accept(), Err(e) if e.kind() == ErrorKind::WouldBlock),
            "the stream endpoint must see no connection (network={network})"
        );
    }
}

#[test]
fn changed_lists_every_touched_path_and_discard_restores_the_worktree() {
    let f = fixture("scope");
    let dir = f.work.prepare(&f.card).expect("prepare");
    assert_eq!(f.work.changed(&dir).expect("clean"), Vec::<String>::new());

    std::fs::write(dir.join("README"), "changed\n").expect("modify");
    std::fs::create_dir_all(dir.join("a/b")).expect("dirs");
    std::fs::write(dir.join("a/b/new.rs"), "x").expect("new file");
    std::fs::write(dir.join("with space.txt"), "x").expect("odd name");
    let mut seen = f.work.changed(&dir).expect("changed");
    seen.sort();
    assert_eq!(
        seen,
        ["README", "a/b/new.rs", "with space.txt"],
        "untracked files are listed one by one"
    );

    f.work.discard(&dir).expect("discard");
    assert_eq!(
        f.work.changed(&dir).expect("clean again"),
        Vec::<String>::new()
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("README")).expect("read"),
        "hi\n"
    );
    assert!(!dir.join("a").exists() && !dir.join("with space.txt").exists());
}

#[test]
fn a_rename_reports_both_paths() {
    let f = fixture("rename");
    let dir = f.work.prepare(&f.card).expect("prepare");
    git(&dir, &["mv", "README", "elsewhere"]);
    let mut seen = f.work.changed(&dir).expect("changed");
    seen.sort();
    assert_eq!(seen, ["README", "elsewhere"], "the path it left counts too");
}

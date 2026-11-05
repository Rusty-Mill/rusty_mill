//! `lel-jail` end to end: the built launcher, the way the mux will run it
//! (LEL D85 tests; mux-hosted-agents-design.md §4).
//!
//! The confined program is this test binary again, run as the `probe` test
//! with `LEL_JAIL_PROBE` set: it reports what it can see and exits.
#![cfg(target_os = "linux")]

use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixDatagram, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const PROBE: &str = "LEL_JAIL_PROBE";

/// Runs only inside the jail; a no-op in an ordinary test run.
#[test]
fn probe() {
    if std::env::var_os(PROBE).is_none() {
        return;
    }
    let open: Vec<String> = (0..256)
        .filter(|&fd| rusty_libc::fd::fcntl(fd, rusty_libc::fd::F_GETFD, 0).is_ok())
        .map(|fd| fd.to_string())
        .collect();
    println!("fds={}", open.join(","));
    let mux = std::env::var_os("RUSTY_MUX_SOCKET").map_or("unset", |_| "set");
    println!("mux={mux}");
    let unix = UnixDatagram::unbound().map_or("refused", |_| "allowed");
    println!("unix={unix}");
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    let line = cgroup
        .lines()
        .find(|l| l.starts_with("0::"))
        .unwrap_or("none");
    println!("cgroup={line}");
    // Seen from the work directory: the dummy tokens, the root's name after
    // a swap to them, and the checked directory under its new name.
    let seen = |p: &str| {
        if std::fs::read_to_string(p).is_ok() {
            "ok"
        } else {
            "denied"
        }
    };
    println!("tokens={}", seen("../tokens/c-1"));
    println!("swapped={}", seen("../shared/tools/c-1"));
    println!("pinned={}", seen("../shared/tools.orig/tool.txt"));
    std::process::exit(0);
}

const LAUNCH: &str = "LEL_JAIL_LAUNCH";
const GATE: &str = "LEL_JAIL_GATE";

/// Runs only as a launcher: `launch_with` on the spec in `LEL_JAIL_LAUNCH`,
/// pausing after the paths are pinned and checked until the test lets it
/// go. A no-op in an ordinary test run.
#[test]
fn launcher_probe() {
    let (Some(spec), Some(gate)) = (std::env::var_os(LAUNCH), std::env::var_os(GATE)) else {
        return;
    };
    let gate = PathBuf::from(gate);
    let error = orch_jail::launch::launch_with(Path::new(&spec), || {
        std::fs::write(gate.join("paused"), "").unwrap();
        while !gate.join("go").exists() {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });
    eprintln!("lel-jail: {error}");
    std::process::exit(125);
}

struct Fixture {
    root: PathBuf,
    cgroup: Option<PathBuf>,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("lel-jail-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("work")).unwrap();
        File::create(root.join("status")).unwrap();
        std::fs::create_dir(root.join("tokens")).unwrap();
        std::fs::write(root.join("tokens/c-1"), "tok-123\n").unwrap();
        Self { root, cgroup: None }
    }

    fn token(&self) -> PathBuf {
        self.root.join("tokens/c-1")
    }

    /// A fresh cgroup under the cgroup2 mount, or why there can't be one.
    fn make_cgroup(&mut self) -> Result<PathBuf, String> {
        let mounts = std::fs::read_to_string("/proc/self/mounts").map_err(|e| e.to_string())?;
        let base = mounts
            .lines()
            .map(|l| l.split(' ').collect::<Vec<_>>())
            .find(|f| f.len() > 2 && f[2] == "cgroup2" && f[1].starts_with("/sys/fs/cgroup"))
            .map(|f| PathBuf::from(f[1]))
            .ok_or("no cgroup2 mount under /sys/fs/cgroup")?;
        let dir = base.join(format!("lel-jail-test-{}", std::process::id()));
        std::fs::create_dir(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
        self.cgroup = Some(dir.clone());
        Ok(dir)
    }

    fn write_spec(&self, cgroup: &Path) -> PathBuf {
        self.write_spec_with(cgroup, &[], &self.token())
    }

    /// The spec with `more` read roots and the token at `token`.
    fn write_spec_with(&self, cgroup: &Path, more: &[PathBuf], token: &Path) -> PathBuf {
        let exe = std::env::current_exe().unwrap();
        let mut read: Vec<String> = ["/usr", "/lib", "/lib64", "/etc", "/proc"]
            .into_iter()
            .filter(|p| Path::new(p).exists())
            .map(String::from)
            .collect();
        read.push(exe.parent().unwrap().display().to_string());
        read.extend(more.iter().map(|p| p.display().to_string()));
        let json = format!(
            r#"{{"cgroup": {cg:?}, "status": {st:?}, "cwd": {wk:?}, "read": {rd:?}, "write": [{wk:?}],
                "limits": {{"cpu_secs": 30, "memory_bytes": 4294967296, "file_bytes": 1048576,
                            "open_files": 256, "processes": 4096}},
                "env": {{"{PROBE}": "1", "PATH": "/usr/bin:/bin"}},
                "token": {{"file": {tk:?}, "var": "LEL_TOKEN"}},
                "program": {exe:?}, "args": ["--exact", "probe", "--nocapture", "--test-threads=1"]}}"#,
            cg = cgroup.display().to_string(),
            st = self.root.join("status").display().to_string(),
            wk = self.root.join("work").display().to_string(),
            rd = read,
            tk = token.display().to_string(),
            exe = exe.display().to_string(),
        );
        let path = self.root.join("jail.json");
        std::fs::write(&path, json).unwrap();
        path
    }

    fn status(&self) -> String {
        std::fs::read_to_string(self.root.join("status")).unwrap_or_default()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(cgroup) = &self.cgroup {
            let _ = std::fs::remove_dir(cgroup);
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Runs these tests one at a time: a launch holds fds 3, 4 and 7 of this
/// process, and an open on another thread would race it for them.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Runs `lel-jail` with `fds` placed at 3, 4, … as the mux will pass them,
/// plus a stray descriptor at 7 that must not reach the agent, and the mux
/// variable set in the launcher's own environment.
///
/// While it spawns, every free one of 3, 4 and 7 is held open here, so std's
/// exec-error pipe can't take a number the child then overwrites.
fn run_jail(spec: &Path, fds: &[i32]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lel-jail"));
    command.arg(spec);
    run_launcher(command, fds)
}

/// Runs `command` as the mux would run the launcher (see [`run_jail`]).
fn run_launcher(mut command: Command, fds: &[i32]) -> Output {
    use rusty_libc::fd::{close, dup2, fcntl, F_DUPFD_CLOEXEC, F_GETFD};
    let stray = File::open("/dev/null").unwrap();
    let held: Vec<i32> = [3, 4, 7]
        .into_iter()
        .filter(|&fd| fcntl(fd, F_GETFD, 0).is_err())
        .inspect(|&fd| {
            dup2(stray.as_raw_fd(), fd).unwrap();
        })
        .collect();
    // Sources above every target, so placing one never clobbers another.
    let high = |fd: i32| fcntl(fd, F_DUPFD_CLOEXEC, 8).unwrap();
    let fds: Vec<i32> = fds.iter().copied().map(high).collect();
    let stray_fd = high(stray.as_raw_fd());
    let sources = fds.clone();
    command.env("RUSTY_MUX_SOCKET", "/run/user/0/rusty-mux/sock");
    // SAFETY: only raw close/dup2 syscalls run between fork and exec.
    unsafe {
        command.pre_exec(move || {
            let failed = |_| std::io::Error::other("placing a descriptor");
            for target in [3, 4] {
                let _ = close(target);
            }
            for (target, &fd) in (3..).zip(&sources) {
                dup2(fd, target).map_err(failed)?;
            }
            dup2(stray_fd, 7).map_err(failed)?;
            Ok(())
        });
    }
    let out = command.output().unwrap();
    for fd in held.into_iter().chain(fds).chain([stray_fd]) {
        close(fd).unwrap();
    }
    out
}

#[test]
fn the_agent_sees_fds_0_to_4_no_mux_and_no_unix_sockets() {
    let _serial = serial();
    let mut fx = Fixture::new("ok");
    let cgroup = match fx.make_cgroup() {
        Ok(cgroup) => cgroup,
        Err(why) => return eprintln!("no writable cgroup; skipping: {why}"),
    };
    let spec = fx.write_spec(&cgroup);
    let slot = File::create(fx.root.join("slot.lock")).unwrap();
    let (acp, _orch) = UnixStream::pair().unwrap();
    let out = run_jail(&spec, &[slot.as_raw_fd(), acp.as_raw_fd()]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let log = format!(
        "{stdout}{}{}",
        String::from_utf8_lossy(&out.stderr),
        fx.status()
    );
    if log.contains("Landlock filesystem confinement is NotEnforced") {
        return eprintln!("kernel without Landlock; skipping: {log}");
    }
    assert!(out.status.success(), "{log}");
    assert!(stdout.contains("fds=0,1,2,3,4\n"), "{log}");
    assert!(stdout.contains("mux=unset\n"), "{log}");
    assert!(stdout.contains("unix=refused\n"), "{log}");
    let name = cgroup.file_name().unwrap().to_string_lossy();
    assert!(
        stdout.contains(&format!("/{name}\n")),
        "joined the cgroup: {log}"
    );
    assert_eq!(
        fx.status(),
        "",
        "a successful exec leaves the status file empty"
    );
}

#[test]
fn a_missing_slot_or_acp_fd_fails_closed_before_anything_runs() {
    let _serial = serial();
    let fx = Fixture::new("nofd");
    // The fd check comes before the cgroup is touched, so any path works.
    let spec = fx.write_spec(Path::new("/sys/fs/cgroup/lel-jail-never"));
    let slot = File::create(fx.root.join("slot.lock")).unwrap();
    for (fds, missing) in [(vec![], "fd 3"), (vec![slot.as_raw_fd()], "fd 4")] {
        let out = run_jail(&spec, &fds);
        assert_eq!(out.status.code(), Some(125), "{out:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(missing), "{stderr}");
        assert!(fx.status().contains(missing), "status: {}", fx.status());
        assert!(out.stdout.is_empty(), "nothing ran: {out:?}");
    }
}

#[test]
fn a_bad_spec_is_refused_with_its_field() {
    let _serial = serial();
    let fx = Fixture::new("bad");
    let path = fx.root.join("jail.json");
    std::fs::write(&path, r#"{"cgroup": "/tmp/x"}"#).unwrap();
    let out = run_jail(&path, &[]);
    assert_eq!(out.status.code(), Some(125));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("jail.json"),
        "{out:?}"
    );
}

/// Runs `lel-jail` with good fds on `spec`; `None` when there is no
/// writable cgroup to join, which comes before the path checks.
fn launch_refused(fx: &mut Fixture, spec: impl Fn(&Fixture, &Path) -> PathBuf) -> Option<String> {
    let cgroup = match fx.make_cgroup() {
        Ok(cgroup) => cgroup,
        Err(why) => {
            eprintln!("no writable cgroup; skipping: {why}");
            return None;
        }
    };
    let spec = spec(fx, &cgroup);
    let slot = File::create(fx.root.join("slot.lock")).unwrap();
    let (acp, _orch) = UnixStream::pair().unwrap();
    let out = run_jail(&spec, &[slot.as_raw_fd(), acp.as_raw_fd()]);
    assert_eq!(out.status.code(), Some(125), "{out:?}");
    assert!(out.stdout.is_empty(), "the agent ran: {out:?}");
    Some(fx.status())
}

/// D85: a root that is a symlink to the token directory, or a token path
/// that is a symlink into a root, passes the spelled-out check but not the
/// resolved one, and no agent starts with the dummy token in reach.
#[test]
fn a_symlink_alias_of_a_root_or_the_token_refuses_the_launch() {
    let _serial = serial();
    let mut fx = Fixture::new("alias-root");
    let status = launch_refused(&mut fx, |fx, cg| {
        let alias = fx.root.join("public");
        std::os::unix::fs::symlink(fx.root.join("tokens"), &alias).unwrap();
        fx.write_spec_with(cg, &[alias], &fx.token())
    });
    if let Some(status) = status {
        assert!(status.contains("a sandbox root reaches"), "{status}");
    }
    drop(fx);
    let mut fx = Fixture::new("alias-token");
    let status = launch_refused(&mut fx, |fx, cg| {
        std::fs::write(fx.root.join("work/c-1"), "tok-123\n").unwrap();
        let alias = fx.root.join("tok");
        std::os::unix::fs::symlink(fx.root.join("work"), &alias).unwrap();
        fx.write_spec_with(cg, &[], &alias.join("c-1"))
    });
    if let Some(status) = status {
        assert!(status.contains("a sandbox root reaches"), "{status}");
    }
}

/// `..` never reaches the resolver: the spec refuses it.
#[test]
fn a_parent_component_in_a_root_refuses_the_launch() {
    let _serial = serial();
    let mut fx = Fixture::new("dotdot");
    let status = launch_refused(&mut fx, |fx, cg| {
        let sneaky = fx.root.join("work/../tokens");
        fx.write_spec_with(cg, &[sneaky], &fx.token())
    });
    if let Some(status) = status {
        assert!(
            status.is_empty(),
            "refused at parse, before the status file: {status}"
        );
    }
}

/// D85 (#1506 review): a root checked and then swapped, before the rules go
/// in, for a symlink to the token directory, as a peer with write access to
/// its parent could do. The rules land on the directory that was checked,
/// so the agent reaches it under its new name and never reaches the tokens.
#[test]
fn a_root_swapped_after_the_check_grants_only_the_checked_directory() {
    let _serial = serial();
    let mut fx = Fixture::new("swap");
    let cgroup = match fx.make_cgroup() {
        Ok(cgroup) => cgroup,
        Err(why) => return eprintln!("no writable cgroup; skipping: {why}"),
    };
    let tools = fx.root.join("shared/tools");
    std::fs::create_dir_all(&tools).unwrap();
    std::fs::write(tools.join("tool.txt"), "tool").unwrap();
    let gate = fx.root.join("gate");
    std::fs::create_dir(&gate).unwrap();
    let spec = fx.write_spec_with(&cgroup, std::slice::from_ref(&tools), &fx.token());
    let slot = File::create(fx.root.join("slot.lock")).unwrap();
    let (acp, _orch) = UnixStream::pair().unwrap();
    let fds = [slot.as_raw_fd(), acp.as_raw_fd()];
    let mut launcher = Command::new(std::env::current_exe().unwrap());
    launcher
        .args([
            "--exact",
            "launcher_probe",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(LAUNCH, &spec)
        .env(GATE, &gate);
    let out = std::thread::scope(|scope| {
        let running = scope.spawn(|| run_launcher(launcher, &fds));
        let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !gate.join("paused").exists() {
            assert!(
                !running.is_finished(),
                "the launcher stopped before the pause"
            );
            assert!(std::time::Instant::now() < end, "the launcher never paused");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // The swap: the checked root becomes a symlink to the tokens.
        std::fs::rename(&tools, fx.root.join("shared/tools.orig")).unwrap();
        std::os::unix::fs::symlink(fx.root.join("tokens"), &tools).unwrap();
        std::fs::write(gate.join("go"), "").unwrap();
        running.join().unwrap()
    });
    let stdout = String::from_utf8_lossy(&out.stdout);
    let log = format!(
        "{stdout}{}{}",
        String::from_utf8_lossy(&out.stderr),
        fx.status()
    );
    if log.contains("Landlock filesystem confinement is NotEnforced") {
        return eprintln!("kernel without Landlock; skipping: {log}");
    }
    assert!(out.status.success(), "{log}");
    assert!(stdout.contains("tokens=denied\n"), "{log}");
    assert!(
        stdout.contains("swapped=denied\n"),
        "the swap redirected a rule: {log}"
    );
    assert!(
        stdout.contains("pinned=ok\n"),
        "the checked root was granted: {log}"
    );
}

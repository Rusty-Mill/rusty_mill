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
    std::process::exit(0);
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
        std::fs::write(root.join("token"), "tok-123\n").unwrap();
        Self { root, cgroup: None }
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
        let exe = std::env::current_exe().unwrap();
        let mut read: Vec<String> = ["/usr", "/lib", "/lib64", "/etc", "/proc"]
            .into_iter()
            .filter(|p| Path::new(p).exists())
            .map(String::from)
            .collect();
        read.push(exe.parent().unwrap().display().to_string());
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
            tk = self.root.join("token").display().to_string(),
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
    let mut command = Command::new(env!("CARGO_BIN_EXE_lel-jail"));
    command
        .arg(spec)
        .env("RUSTY_MUX_SOCKET", "/run/user/0/rusty-mux/sock");
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

//! C69 slice (a): with no socket named, the server listens on the default
//! socket under `$XDG_RUNTIME_DIR`, in a directory private to the user, and
//! a second server refuses to take a live server's name; with
//! `--idle-exit` it stops once it has no block and no client, never while a
//! block lives.

#![cfg(target_os = "linux")]

mod common;

use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const PATIENCE: Duration = Duration::from_secs(10);

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rmx-default-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).expect("scratch");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    dir
}

/// A server with no socket named, under `runtime` as `$XDG_RUNTIME_DIR`.
fn server(runtime: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rusty-mux-server"));
    command
        .arg("--no-state")
        .env("XDG_RUNTIME_DIR", runtime)
        .env_remove("RUSTY_MUX_SOCKET")
        .env("SHELL", "/bin/sh")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
}

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_for(socket: &Path) {
    let deadline = Instant::now() + PATIENCE;
    while UnixStream::connect(socket).is_err() {
        assert!(Instant::now() < deadline, "server never listened");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_default_socket_is_private_and_a_second_server_refuses_it() {
    let runtime = scratch("private");
    let socket = runtime.join("rusty_mux").join("default.sock");
    let _first = Running(server(&runtime).spawn().expect("first"));
    wait_for(&socket);
    let dir_mode = std::fs::metadata(runtime.join("rusty_mux"))
        .expect("dir")
        .mode();
    assert_eq!(dir_mode & 0o777, 0o700);
    let sock_mode = std::fs::symlink_metadata(&socket).expect("socket").mode();
    assert_eq!(sock_mode & 0o777, 0o600);

    let second = server(&runtime).output().expect("second");
    assert!(!second.status.success(), "the second server must refuse");
    let said = String::from_utf8_lossy(&second.stderr);
    assert!(said.contains("already answers"), "{said}");
    UnixStream::connect(&socket).expect("the first still answers");
    let entries = std::fs::read_dir(runtime.join("rusty_mux"))
        .expect("dir")
        .count();
    assert_eq!(entries, 1, "no temporary name left behind");
    let _ = std::fs::remove_dir_all(&runtime);
}

#[test]
fn a_server_replaces_a_socket_left_by_a_killed_one() {
    let runtime = scratch("stale");
    let socket = runtime.join("rusty_mux").join("default.sock");
    let first = Running(server(&runtime).spawn().expect("first"));
    wait_for(&socket);
    drop(first);
    assert!(socket.exists(), "a killed server leaves its name");
    let _second = Running(server(&runtime).spawn().expect("second"));
    wait_for(&socket);
    let _ = std::fs::remove_dir_all(&runtime);
}

#[test]
fn an_open_default_directory_is_refused() {
    let runtime = scratch("open");
    let dir = runtime.join("rusty_mux");
    std::fs::create_dir(&dir).expect("dir");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let output = server(&runtime).output().expect("server");
    assert!(!output.status.success());
    assert!(!dir.join("default.sock").exists());
    let _ = std::fs::remove_dir_all(&runtime);
}

/// Waits up to `limit` for `child` to exit, returning whether it did.
fn exits_within(child: &mut Child, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if child.try_wait().expect("try_wait").is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn idle_exit_waits_for_the_last_block_and_client_then_takes_its_name_away() {
    let runtime = scratch("idle");
    let socket = runtime.join("rusty_mux").join("default.sock");
    let mut running = Running(
        server(&runtime)
            .args(["--idle-exit", "1"])
            .spawn()
            .expect("server"),
    );
    wait_for(&socket);

    let mut client = common::Client::connect(&socket);
    let block = common::block_for(&mut client);
    drop(client);
    assert!(
        !exits_within(&mut running.0, Duration::from_secs(3)),
        "a live block keeps the server"
    );

    let mut client = common::Client::connect(&socket);
    client.request(|req| rusty_mux_proto::Message::CloseBlock { req, block });
    drop(client);
    assert!(
        exits_within(&mut running.0, Duration::from_secs(10)),
        "no block and no client: the server stops"
    );
    let status = running.0.wait().expect("status");
    assert!(status.success(), "a clean stop: {status}");
    assert!(!socket.exists(), "a clean stop takes its own name away");
    let _ = std::fs::remove_dir_all(&runtime);
}

#[test]
fn a_socket_from_systemd_is_served_and_its_name_left_alone() {
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;

    let runtime = scratch("activated");
    let socket = runtime.join("activated.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).expect("bind");
    let fd = listener.as_raw_fd();
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        // `exec` keeps the shell's pid, so `$$` is the server's.
        .arg(r#"LISTEN_PID=$$ LISTEN_FDS=1 exec "$0" --no-state --idle-exit 1"#)
        .arg(env!("CARGO_BIN_EXE_rusty-mux-server"))
        .env_remove("RUSTY_MUX_SOCKET")
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("SHELL", "/bin/sh")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: `dup2` is async-signal-safe and allocates nothing; it gives
    // the child the listener as fd 3, as systemd does.
    unsafe {
        command.pre_exec(move || {
            // `dup2` onto itself keeps close-on-exec, so clear it too.
            const F_SETFD: i32 = 2;
            let errno = |e: rusty_libc::Errno| std::io::Error::from_raw_os_error(e.0);
            rusty_libc::fd::dup2(fd, 3).map_err(errno)?;
            rusty_libc::fd::fcntl(3, F_SETFD, 0)
                .map(|_| ())
                .map_err(errno)
        });
    }
    let mut running = Running(command.spawn().expect("server"));
    drop(listener);

    let mut client = common::Client::connect(&socket);
    let _block = common::block_for(&mut client);
    drop(client);
    assert!(
        close_on_exec(running.0.id(), 3),
        "the listener is close-on-exec, so blocks' programs don't inherit it"
    );
    assert!(
        !runtime.join("rusty_mux").join("default.sock").exists(),
        "an activated server binds nothing of its own"
    );
    // Its blocks still live, so it keeps running; then stop it.
    assert!(!exits_within(&mut running.0, Duration::from_millis(1500)));
    drop(running);
    assert!(socket.exists(), "the name stays systemd's");
    let _ = std::fs::remove_dir_all(&runtime);
}

/// Whether `pid`'s descriptor `fd` is close-on-exec (`O_CLOEXEC` in the
/// octal `flags:` line of its fdinfo).
fn close_on_exec(pid: u32, fd: u32) -> bool {
    const O_CLOEXEC: u32 = 0o2_000_000;
    let info = std::fs::read_to_string(format!("/proc/{pid}/fdinfo/{fd}")).expect("fdinfo");
    let flags = info
        .lines()
        .find_map(|line| line.strip_prefix("flags:"))
        .and_then(|octal| u32::from_str_radix(octal.trim(), 8).ok())
        .expect("flags");
    flags & O_CLOEXEC != 0
}

//! C69 slice (a): with no socket named, the server listens on the default
//! socket under `$XDG_RUNTIME_DIR`, in a directory private to the user, and
//! a second server refuses to take a live server's name.

#![cfg(target_os = "linux")]

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

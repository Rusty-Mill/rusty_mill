//! Without `runtime::install_fallback`, rusty_tokio still refuses to run off
//! its own runtime. A separate binary from `tests/fallback.rs`, which
//! installs it for the whole process.

#[test]
#[should_panic(expected = "install_fallback")]
fn spawn_without_a_runtime_or_fallback_panics() {
    drop(rusty_tokio::spawn(async {}));
}

#[test]
#[should_panic(expected = "no rusty_tokio runtime")]
fn sleep_without_a_runtime_or_fallback_panics() {
    drop(rusty_tokio::time::sleep(std::time::Duration::from_millis(
        1,
    )));
}

/// With nothing to reap on, dropping a `kill_on_drop` child still kills it
/// and doesn't panic in `Drop`: the reap is skipped, leaving a zombie the
/// test reaps itself. Passes before #1302's fix too: it guards against a
/// fix that reaches the fallback through the panicking lookup, and fails
/// if `Child`'s `Drop` uses `Handle::current_or_fallback()` (checked).
#[cfg(unix)]
#[test]
fn kill_on_drop_without_a_runtime_or_fallback_kills_without_panicking() {
    use std::time::{Duration, Instant};

    let child = rusty_tokio::process::Command::new("sleep")
        .arg("30")
        .kill_on_drop(true)
        .spawn()
        .expect("spawn");
    let pid = child.id() as libc::pid_t;
    drop(child);

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut status = 0;
    loop {
        // SAFETY: `pid` is this process's own child; WNOHANG never blocks.
        let reaped = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        assert!(reaped >= 0, "waitpid: {}", std::io::Error::last_os_error());
        if reaped == pid {
            break;
        }
        if Instant::now() >= deadline {
            // SAFETY: as above; don't leave a running child behind.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
                libc::waitpid(pid, std::ptr::null_mut(), 0);
            }
            panic!("dropped kill_on_drop child {pid} was not killed");
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    assert!(libc::WIFSIGNALED(status) && libc::WTERMSIG(status) == libc::SIGKILL);
}

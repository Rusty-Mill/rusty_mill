//! `runtime::install_fallback`: rusty_tokio I/O, timers and `spawn` polled by
//! another executor. Its own test binary, because the fallback is
//! process-wide and every test here installs it.

use rusty_tokio::io::{TcpListener, TcpStream};
use rusty_tokio::runtime::install_fallback;
use std::time::{Duration, Instant};

/// Runs `future` on a tokio current-thread runtime: the foreign executor
/// this feature exists for.
fn on_tokio<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("tokio runtime")
        .block_on(future)
}

#[test]
fn tcp_round_trips_on_a_tokio_thread() {
    install_fallback().expect("fallback");
    on_tokio(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let client = TcpStream::connect(addr).await.expect("connect");
        let (server, _) = listener.accept().await.expect("accept");

        client.write_all(b"ping").await.expect("write");
        let mut buf = [0u8; 4];
        server.read_exact(&mut buf).await.expect("read");

        assert_eq!(&buf, b"ping");
    });
}

#[test]
fn sleep_fires_on_a_tokio_thread() {
    install_fallback().expect("fallback");
    let started = Instant::now();

    // Built inside the tokio task, so the timer is looked up on a tokio
    // thread rather than on this bare test thread.
    on_tokio(async { rusty_tokio::time::sleep(Duration::from_millis(50)).await });

    assert!(started.elapsed() >= Duration::from_millis(50));
}

#[test]
fn timeout_elapses_on_a_tokio_thread() {
    install_fallback().expect("fallback");

    let outcome = on_tokio(async {
        rusty_tokio::time::timeout(Duration::from_millis(20), std::future::pending::<()>()).await
    });

    assert!(outcome.is_err());
}

#[test]
fn spawned_tasks_run_on_the_fallback_worker() {
    install_fallback().expect("fallback");

    let name = on_tokio(async {
        rusty_tokio::spawn(async { std::thread::current().name().map(str::to_owned) }).await
    })
    .expect("join");
    let blocking = on_tokio(async { rusty_tokio::spawn_blocking(|| 6 * 7).await }).expect("join");

    assert_eq!(name.as_deref(), Some("rusty-tokio-fallback"));
    assert_eq!(blocking, 42);
}

#[test]
fn a_rusty_tokio_runtime_keeps_its_own_tasks() {
    install_fallback().expect("fallback");
    let runtime = rusty_tokio::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("own-runtime")
        .build()
        .expect("runtime");

    // Spawned inside `block_on`: outside it this thread has no runtime and
    // the task would go to the fallback.
    let name = runtime
        .block_on(async {
            rusty_tokio::spawn(async { std::thread::current().name().map(str::to_owned) }).await
        })
        .expect("join");

    assert_eq!(name.as_deref(), Some("own-runtime"));
}

#[test]
fn work_built_on_a_bare_thread_also_uses_the_fallback() {
    install_fallback().expect("fallback");
    let sleep = rusty_tokio::time::sleep(Duration::from_millis(10));
    let task = rusty_tokio::spawn(async { 7 });

    on_tokio(sleep);

    assert_eq!(on_tokio(task).expect("join"), 7);
}

#[test]
fn current_still_reports_only_the_threads_own_runtime() {
    install_fallback().expect("fallback");

    assert!(rusty_tokio::Handle::try_current().is_none());
}

#[test]
fn installing_twice_returns_the_same_runtime() {
    let first = install_fallback().expect("fallback");
    let second = install_fallback().expect("fallback");

    assert_eq!(first.id(), second.id());
}

/// #1302: a `kill_on_drop` child dropped on a tokio thread is killed *and*
/// reaped through the fallback, not left a zombie because the thread has no
/// rusty_tokio runtime of its own.
#[cfg(unix)]
#[test]
fn kill_on_drop_child_dropped_on_a_tokio_thread_is_reaped() {
    use rusty_tokio::process::{Command, Stdio};

    install_fallback().expect("fallback");
    let pid = on_tokio(async {
        let child = Command::new("sleep")
            .arg("30")
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn");
        let pid = child.id() as libc::pid_t;
        drop(child);
        pid
    });

    // `kill(pid, 0)` still succeeds on a zombie; ESRCH means it was reaped.
    let deadline = Instant::now() + Duration::from_secs(5);
    let reaped = loop {
        // SAFETY: signal 0 only checks that `pid` exists; nothing is sent.
        if unsafe { libc::kill(pid, 0) } == -1
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // Linux shows a zombie as state `Z`; elsewhere this is just `None`.
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok();
    if !reaped {
        // Don't leave the zombie (or a still-running child) behind.
        // SAFETY: `pid` is this process's own child, not yet reaped.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
            libc::waitpid(pid, std::ptr::null_mut(), 0);
        }
    }

    assert!(
        reaped,
        "dropped kill_on_drop child {pid} was not reaped: {stat:?}"
    );
}

use std::time::Duration;

use rusty_tokio::{Builder, LocalOptions};

use super::*;

/// A peer whose uid is not the owner's is closed before any reply.
#[cfg(target_os = "linux")]
#[test]
fn a_client_of_another_user_is_refused() {
    let path = std::env::temp_dir().join(format!("rusty-mux-uid-{}.sock", std::process::id()));
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let bind_path = path.clone();
    std::thread::spawn(move || {
        let runtime = Builder::new_current_thread()
            .build_local(&LocalOptions::default())
            .expect("runtime");
        runtime.block_on(async move {
            let listener = UnixListener::bind(&bind_path).expect("bind");
            ready_tx.send(()).expect("signal ready");
            let config = Config {
                program: "/bin/sh".into(),
                args: Vec::new(),
                cwd: "/".into(),
                console_host: ConsoleHost::default(),
            };
            let other = rusty_libc::process::geteuid().wrapping_add(1);
            serve_for(listener, config, None, |s: &UnixStream| owned_by(s, other))
                .await
                .expect("serve");
        });
    });
    ready_rx
        .recv_timeout(Duration::from_secs(20))
        .expect("server bound");

    use std::io::Read;
    let mut client = std::os::unix::net::UnixStream::connect(&path).expect("connect");
    client
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("timeout");
    let mut buf = [0u8; 16];
    // The server drops the stream unread: end of stream, never a Welcome.
    assert_eq!(client.read(&mut buf).expect("read"), 0);
    let _ = std::fs::remove_file(&path);
}

/// A connection from this process passes the Windows peer check.
#[cfg(windows)]
#[test]
fn a_client_of_this_process_is_allowed() {
    let path = std::env::temp_dir().join(format!("rusty-mux-sid-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let runtime = Builder::new_current_thread()
        .build_local(&LocalOptions::default())
        .expect("runtime");
    runtime.block_on(async {
        let listener = UnixListener::bind(&path).expect("bind");
        let connect_path = path.clone();
        let client = std::thread::spawn(move || {
            platform_windows::WindowsUnixStream::connect(&connect_path).expect("connect")
        });
        let (stream, _) = rusty_tokio::time::timeout(Duration::from_secs(20), listener.accept())
            .await
            .expect("accepted in time")
            .expect("accept");
        assert!(matches!(peer_check(&stream).expect("check"), Peer::Allowed));
        drop(client.join().expect("client"));
    });
    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_console_host_defaults_to_conpty_and_parses_in_any_case() {
    use std::ffi::OsStr;
    let parse = |v: &str| ConsoleHost::parse(Some(OsStr::new(v)));
    assert_eq!(ConsoleHost::parse(None), Ok(ConsoleHost::ConPty));
    assert_eq!(parse("conpty"), Ok(ConsoleHost::ConPty));
    assert_eq!(parse("Own"), Ok(ConsoleHost::Own));
    assert_eq!(parse("OWN"), Ok(ConsoleHost::Own));
}

/// No silent fallback: a value that names no host is refused, and named.
#[test]
fn an_unknown_console_host_is_an_error() {
    use std::ffi::OsStr;
    for value in ["", "openconsole", " own"] {
        let err = ConsoleHost::parse(Some(OsStr::new(value))).expect_err(value);
        assert!(
            err.contains(&format!("RUSTY_CONSOLE_HOST={value}:")),
            "{err}"
        );
    }
}

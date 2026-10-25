use std::io::Read;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use rusty_tokio::{Builder, LocalOptions};

use super::*;

/// A peer whose uid is not the owner's is closed before any reply.
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
            };
            let other = rusty_libc::process::geteuid().wrapping_add(1);
            serve_for(listener, config, other).await.expect("serve");
        });
    });
    ready_rx
        .recv_timeout(Duration::from_secs(20))
        .expect("server bound");

    let mut client = UnixStream::connect(&path).expect("connect");
    client
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("timeout");
    let mut buf = [0u8; 16];
    // The server drops the stream unread: end of stream, never a Welcome.
    assert_eq!(client.read(&mut buf).expect("read"), 0);
    let _ = std::fs::remove_file(&path);
}

//! A clean stop (`serve_until`'s future resolving) ends every block's
//! program and returns. Windows' own-host survivors are covered end to end
//! in `windows.rs`.

#![cfg(target_os = "linux")]

mod common;

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use common::{Client, PATIENCE, block_for, shell, window_for};
use rusty_mux_core::{BlockId, Size};
use rusty_mux_proto::{Features, Launch, Message};
use rusty_mux_server::serve_until;
use rusty_tokio::io::UnixListener;
use rusty_tokio::sync::oneshot;
use rusty_tokio::{Builder, LocalOptions};

/// A server on a fresh socket that stops when the sender is used; the
/// receiver gets `serve_until`'s result.
fn stoppable(name: &str) -> (PathBuf, oneshot::Sender<()>, Receiver<Result<(), String>>) {
    let path = std::env::temp_dir().join(format!("rusty-mux-{name}-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let (stop, stopped) = oneshot::channel::<()>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (ended_tx, ended_rx) = std::sync::mpsc::channel();
    let bind_path = path.clone();
    std::thread::spawn(move || {
        let runtime = Builder::new_current_thread()
            .build_local(&LocalOptions::default())
            .expect("runtime");
        let ended = runtime.block_on(async move {
            let listener = UnixListener::bind(&bind_path).expect("bind");
            ready_tx.send(()).expect("signal ready");
            let stop = async {
                let _ = stopped.await;
            };
            serve_until(listener, shell(), None, stop).await
        });
        let _ = ended_tx.send(ended.map_err(|e| e.to_string()));
    });
    ready_rx.recv_timeout(PATIENCE).expect("server bound");
    (path, stop, ended_rx)
}

const OWNER: Features = Features::CHECKPOINT_V6
    .union(Features::LAUNCH)
    .union(Features::OWNERSHIP);

#[test]
fn a_clean_stop_ends_every_block_and_returns() {
    let (path, stop, ended_rx) = stoppable("stop");

    let mut a = Client::connect(&path);
    let shells: Vec<u32> = (0..2)
        .map(|_| {
            let block = block_for(&mut a);
            a.input(block, b"echo SHELL=$$\r");
            a.wait_for_number(block, "SHELL=")
        })
        .collect();
    assert!(shells.iter().all(|&pid| alive(pid)), "{shells:?}");

    let started = Instant::now();
    stop.send(()).expect("the server is listening for a stop");
    let ended = ended_rx
        .recv_timeout(PATIENCE)
        .expect("the server returned");
    assert_eq!(ended, Ok(()));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "stopping took {:?} with nothing to wait for",
        started.elapsed()
    );
    for pid in shells {
        assert!(!alive(pid), "shell {pid} outlived the stop");
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_stop_only_hangs_up_an_owned_block() {
    let (path, stop, ended_rx) = stoppable("stop-owned");
    let mut owner = Client::connect_with(&path, OWNER, None);
    let window = window_for(&mut owner);
    let size = Size::new(80, 24).expect("size");
    let block = BlockId::from_uuid(
        owner
            .request(|req| Message::CreateBlock {
                req,
                window,
                size,
                launch: Launch {
                    own: true,
                    ..Launch::default()
                },
            })
            .expect("a block id"),
    );
    owner.request(|req| Message::Attach { req, block });
    owner.request(|req| Message::AcquireLease { req, block });
    // A program that outlives a hang-up, as an agent under its jail does.
    owner.input(block, b"trap '' HUP; echo SHELL=$$\r");
    let shell = owner.wait_for_number(block, "SHELL=");

    stop.send(()).expect("the server is listening for a stop");
    let ended = ended_rx
        .recv_timeout(PATIENCE)
        .expect("the server returned");
    assert_eq!(ended, Ok(()));
    std::thread::sleep(Duration::from_millis(1500));
    let survived = alive(shell);
    let _ = std::process::Command::new("kill")
        .arg("-KILL")
        .arg(shell.to_string())
        .status();
    assert!(survived, "the owned block's shell was killed by the stop");
    let _ = std::fs::remove_file(&path);
}

/// Whether process `pid` exists; a reaped one has no `/proc` entry.
fn alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

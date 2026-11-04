//! C45: a server started on the store an earlier one saved restores its
//! sessions, windows, layouts and screens, by id, each block with a fresh
//! shell; an unreadable store is moved aside and the server starts empty.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{Client, PATIENCE, block_for, run_and_wait, shell};
use rusty_mux_core::EntityId;
use rusty_mux_proto::{Entry, Features, Message};
use rusty_mux_server::{Config, serve_until};
use rusty_tokio::io::UnixListener;
use rusty_tokio::sync::oneshot;
use rusty_tokio::{Builder, LocalOptions};

/// A running server whose stop is `stop`, and the thread serving it.
struct Server {
    socket: PathBuf,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    fn start(state: &Path) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let socket =
            std::env::temp_dir().join(format!("rusty-mux-restore-{}-{n}.sock", std::process::id()));
        let config = Config {
            state_dir: Some(state.to_path_buf()),
            ..shell()
        };
        let (stop, stopped) = oneshot::channel::<()>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let bind = socket.clone();
        let thread = std::thread::spawn(move || {
            let runtime = Builder::new_current_thread()
                .build_local(&LocalOptions::default())
                .expect("runtime");
            runtime.block_on(async move {
                let listener = UnixListener::bind(&bind).expect("bind");
                ready_tx.send(()).expect("ready");
                let stop = async {
                    let _ = stopped.await;
                };
                serve_until(listener, config, None, stop)
                    .await
                    .expect("serve");
            });
        });
        ready_rx.recv_timeout(PATIENCE).expect("server bound");
        Self {
            socket,
            stop: Some(stop),
            thread: Some(thread),
        }
    }

    /// A clean stop, which saves, waited for.
    fn stop(mut self) {
        let _ = self.stop.take().expect("running").send(());
        self.thread
            .take()
            .expect("running")
            .join()
            .expect("server thread");
        let _ = std::fs::remove_file(&self.socket);
    }
}

fn state_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rusty-mux-state-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn listing(client: &mut Client) -> Vec<Entry> {
    client
        .answer(|req| Message::List { req })
        .into_iter()
        .filter_map(|m| match m {
            Message::Entry { entry, .. } => Some(entry),
            _ => None,
        })
        .collect()
}

fn inspecting(socket: &Path) -> Client {
    Client::connect_with(
        socket,
        Features::CHECKPOINT_V6
            .union(Features::INSPECT)
            .union(Features::LABELS),
        None,
    )
}

#[test]
fn sessions_and_screens_survive_a_restart() {
    let dir = state_dir("restart");
    let server = Server::start(&dir);
    let mut client = inspecting(&server.socket);
    let block = block_for(&mut client);
    run_and_wait(&mut client, block, 21);
    client.request(|req| Message::Rename {
        req,
        target: EntityId::Block(block),
        name: "kept".into(),
    });
    let before = listing(&mut client);
    drop(client);
    server.stop();

    let server = Server::start(&dir);
    let mut client = inspecting(&server.socket);
    assert_eq!(
        names(&listing(&mut client)),
        names(&before),
        "same tree, same ids"
    );
    client.request(|req| Message::Attach { req, block });
    client.wait_ready(block);
    let text = client.text(block);
    assert!(text.contains("MARK42"), "saved screen kept: {text}");
    assert!(text.contains("restored after a server restart"), "{text}");
    client.request(|req| Message::AcquireLease { req, block });
    run_and_wait(&mut client, block, 22);
    drop(client);
    server.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Ids and names, which a restore keeps; not the title or lease holder.
fn names(entries: &[Entry]) -> Vec<(String, String)> {
    entries
        .iter()
        .filter_map(|e| match e {
            Entry::Session { session, name, .. } => Some((session.to_string(), name.clone())),
            Entry::Window { window, name, .. } => Some((window.to_string(), name.clone())),
            Entry::Block { block, name, .. } => Some((block.to_string(), name.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn an_unreadable_store_is_set_aside() {
    let dir = state_dir("bad");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest"), "rusty_mux state 99\n").unwrap();
    let server = Server::start(&dir);
    let mut client = inspecting(&server.socket);
    assert!(
        listing(&mut client)
            .iter()
            .all(|e| matches!(e, Entry::Client { .. }))
    );
    let aside: Vec<_> = std::fs::read_dir(dir.parent().unwrap())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with(&format!(
                "{}.bad-",
                dir.file_name().unwrap().to_string_lossy()
            ))
        })
        .map(|e| e.path())
        .collect();
    assert_eq!(aside.len(), 1, "moved aside once");
    assert!(aside[0].join("manifest").exists());
    drop(client);
    server.stop();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&aside[0]);
}

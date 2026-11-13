//! Shared mode (C26, D232): a server that also serves the users an
//! allowlist names. This process is the server's own user, who is always
//! served; a second user needs a second uid, so what a guest may do is
//! tested in the model (`rusty_mux_core`) and the list in the module's
//! own tests. Here: the server's user is served, a bad list stops it, and
//! a reload answers whether the list took.

#![cfg(target_os = "linux")]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{Client, PATIENCE};
use rusty_mux_proto::{Features, Message};
use rusty_mux_server::{Config, Reload, Sharing, serve_shared};
use rusty_tokio::io::UnixListener;
use rusty_tokio::sync::{mpsc, oneshot};
use rusty_tokio::{Builder, LocalOptions};

struct Shared {
    socket: PathBuf,
    reload: mpsc::UnboundedSender<Reload>,
    stop: Option<oneshot::Sender<()>>,
    ended: std::sync::mpsc::Receiver<std::io::Result<()>>,
}

fn scratch() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rusty-mux-shared-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).expect("scratch");
    dir
}

fn write_list(dir: &Path, text: &str) -> PathBuf {
    let path = dir.join("allow");
    std::fs::write(&path, text).expect("list");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    path
}

fn config() -> Config {
    Config {
        program: "/bin/sh".into(),
        args: Vec::new(),
        cwd: "/".into(),
        console_host: rusty_mux_server::ConsoleHost::default(),
        state_dir: None,
        grapheme_clustering: false,
        config_file: None,
    }
}

/// The client is being answered: a listing is not refused.
fn assert_served(client: &mut Client) {
    let answers = client.answer(|req| Message::List { req });
    assert!(
        !answers.iter().any(|m| matches!(m, Message::Failed { .. })),
        "{answers:?}"
    );
}

impl Shared {
    fn start(dir: &Path, allow: PathBuf) -> Self {
        let socket = dir.join("shared.sock");
        let (reload, reloads) = mpsc::unbounded_channel();
        let (stop, stopped) = oneshot::channel::<()>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (ended_tx, ended) = std::sync::mpsc::channel();
        let bind = socket.clone();
        std::thread::spawn(move || {
            let runtime = Builder::new_current_thread()
                .build_local(&LocalOptions::default())
                .expect("runtime");
            let result = runtime.block_on(async move {
                let listener = UnixListener::bind(&bind).expect("bind");
                ready_tx.send(()).expect("ready");
                serve_shared(listener, config(), Sharing { allow }, reloads, async {
                    let _ = stopped.await;
                })
                .await
            });
            let _ = ended_tx.send(result);
        });
        ready_rx.recv_timeout(PATIENCE).expect("bound");
        Self {
            socket,
            reload,
            stop: Some(stop),
            ended,
        }
    }

    /// Reads the list again; whether it took.
    fn reload(&self) -> std::io::Result<()> {
        let (answer, answered) = oneshot::channel();
        self.reload.send(answer).expect("server running");
        Builder::new_current_thread()
            .build_local(&LocalOptions::default())
            .expect("runtime")
            .block_on(async { rusty_tokio::time::timeout(PATIENCE, answered).await })
            .expect("answered in time")
            .expect("answered")
    }

    fn stop(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.ended
            .recv_timeout(PATIENCE)
            .expect("stopped")
            .expect("ended cleanly");
    }
}

#[test]
fn the_servers_own_user_is_served_with_an_empty_list() {
    let dir = scratch();
    let shared = Shared::start(&dir, write_list(&dir, "# nobody else\n"));
    let mut client = Client::connect_with(
        &shared.socket,
        Features::CHECKPOINT_V6.union(Features::INSPECT),
        None,
    );
    assert_served(&mut client);
    drop(client);
    shared.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_bad_list_stops_the_server_before_it_serves() {
    let dir = scratch();
    let shared = Shared::start(&dir, write_list(&dir, "everyone\n"));
    let ended = shared.ended.recv_timeout(PATIENCE).expect("ended");
    let err = ended.expect_err("a bad list was served");
    assert!(err.to_string().contains("no wildcard"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_list_others_can_write_stops_the_server() {
    let dir = scratch();
    let list = write_list(&dir, "");
    std::fs::set_permissions(&list, std::fs::Permissions::from_mode(0o666)).expect("chmod");
    let shared = Shared::start(&dir, list);
    let ended = shared.ended.recv_timeout(PATIENCE).expect("ended");
    let err = ended.expect_err("a writable list was served");
    assert!(err.to_string().contains("no one else can write"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_reload_answers_whether_the_list_took_and_a_bad_one_changes_nothing() {
    let dir = scratch();
    let list = write_list(&dir, "uid:4000000\n");
    let shared = Shared::start(&dir, list.clone());
    write_list(&dir, "uid:4000000\nuid:4000001\n");
    shared.reload().expect("a good list took");
    write_list(&dir, "*\n");
    let err = shared.reload().expect_err("a bad list was taken");
    assert!(err.to_string().contains("no wildcard"), "{err}");
    let mut client = Client::connect_with(
        &shared.socket,
        Features::CHECKPOINT_V6.union(Features::INSPECT),
        None,
    );
    assert_served(&mut client);
    drop(client);
    shared.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

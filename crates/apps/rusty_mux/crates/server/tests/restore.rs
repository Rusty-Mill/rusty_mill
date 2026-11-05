//! C45: a server started on the store an earlier one saved restores its
//! sessions, windows, layouts and screens, by id: a shell block with a
//! fresh shell, a command block read-only (D72). An unreadable store is
//! moved aside and the server starts empty. C70: a block's history past
//! its scrollback is spooled to the store and read with `terminal.history`.

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
    std::fs::write(dir.join("sessions.mmap"), b"not a slot file").unwrap();
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
    assert!(aside[0].join("sessions.mmap").exists());
    drop(client);
    server.stop();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&aside[0]);
}

/// A block started with a command of its own is not run again: it comes
/// back read-only, showing what it printed, and refuses input until closed,
/// through any number of restarts.
#[cfg(unix)]
#[test]
fn a_command_block_comes_back_read_only() {
    restarted_read_only("command", "echo CMD$((3*7)); exec sleep 600", &["CMD21"]);
}

/// A full-screen command's alternate screen is kept, with the main screen
/// it covered.
#[cfg(unix)]
#[test]
fn a_full_screen_command_keeps_its_screen() {
    let script = r#"echo M$((5*5)); printf "\033[?1049hALT$((3*7))"; exec sleep 600"#;
    restarted_read_only("full-screen", script, &["M25", "ALT21"]);
}

/// The marker goes below the last row, so a cursor left above saved text
/// overwrites none of it.
#[cfg(unix)]
#[test]
fn a_cursor_above_saved_text_overwrites_none_of_it() {
    let script =
        r#"printf "R%s\n" $((10+1)) $((10+2)); printf "R$((10+3))\033[1;1H"; exec sleep 600"#;
    restarted_read_only("cursor", script, &["R11", "R12", "R13"]);
}

/// Starts `script` in a block, waits for the last of `needles`, then
/// restarts the server twice: each time the block shows every one of
/// `needles` once, a single read-only marker,
/// and refuses input.
/// A block running `sh -c script` in a new window, attached. The client
/// needs [`Features::LAUNCH`].
#[cfg(unix)]
fn command_block(client: &mut Client, script: &str) -> rusty_mux_core::BlockId {
    use rusty_mux_proto::{Launch, Program};
    let window = common::window_for(client);
    let launch = Launch {
        program: Some(Program {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
        }),
        cwd: None,
    };
    let block = rusty_mux_core::BlockId::from_uuid(
        client
            .request(|req| Message::CreateBlock {
                req,
                window,
                size: rusty_mux_core::Size::new(80, 24).unwrap(),
                launch,
            })
            .expect("id"),
    );
    client.request(|req| Message::Attach { req, block });
    block
}

#[cfg(unix)]
fn restarted_read_only(name: &str, script: &str, needles: &[&str]) {
    let dir = state_dir(name);
    let server = Server::start(&dir);
    let mut client = Client::connect_with(
        &server.socket,
        Features::CHECKPOINT_V6.union(Features::LAUNCH),
        None,
    );
    let block = command_block(&mut client, script);
    // The command may print before the attach, into the checkpoint.
    client.wait_ready(block);
    let deadline = std::time::Instant::now() + PATIENCE;
    // The last is printed last; a full-screen command hides the rest.
    let last = needles.last().expect("a needle");
    while !client.text(block).contains(last) {
        assert!(std::time::Instant::now() < deadline, "{last} not shown");
        client.recv();
    }
    drop(client);
    server.stop();

    for run in 0..2 {
        let server = Server::start(&dir);
        let mut client = Client::connect(&server.socket);
        client.request(|req| Message::Attach { req, block });
        client.wait_ready(block);
        let text = client.text(block);
        for needle in needles {
            assert_eq!(
                text.matches(needle).count(),
                1,
                "{needle}, run {run}: {text}"
            );
        }
        assert_eq!(
            text.matches("read-only").count(),
            1,
            "one marker, run {run}: {text}"
        );
        client.request(|req| Message::AcquireLease { req, block });
        client.send(&Message::Input {
            req: 900,
            block,
            data: b"echo nope\r".to_vec(),
        });
        let refused = loop {
            match client.recv() {
                Message::Failed {
                    req: 900, reason, ..
                } => break reason,
                Message::Done { req: 900, .. } => panic!("input to a read-only block taken"),
                _ => {}
            }
        };
        assert!(refused.contains("read-only"), "{refused}");
        drop(client);
        server.stop();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
fn history(client: &mut Client, block: rusty_mux_core::BlockId, n: u32, skip: u32) -> String {
    use rusty_mux_proto::schema::Value;
    let args = vec![
        ("history".to_owned(), Value::Lines(n)),
        ("skip".to_owned(), Value::Lines(skip)),
    ];
    match client
        .answer(|req| Message::BlockCall {
            req,
            block,
            method: "terminal.history".into(),
            args,
        })
        .as_slice()
    {
        [Message::Text { text, .. }] => text.clone(),
        other => panic!("expected Text, got {other:?}"),
    }
}

/// Lines past the 10,000 of scrollback go to the spool, survive a restart
/// and go with their block.
#[cfg(unix)]
#[test]
fn history_past_scrollback_is_spooled_and_survives_a_restart() {
    let dir = state_dir("spool");
    let features = Features::CHECKPOINT_V6
        .union(Features::BLOCK_CALL)
        .union(Features::TERMINAL_HISTORY);
    let server = Server::start(&dir);
    let mut client = Client::connect_with(&server.socket, features, None);
    let block = block_for(&mut client);
    client.input(block, b"seq 12000 | sed 's/^/L/'; echo DONE$((6*7))\r");
    client.wait_for(block, "DONE42");
    let all = history(&mut client, block, 100_000, 0);
    let lines: Vec<&str> = all.lines().collect();
    assert!(
        lines.contains(&"L2") && lines.contains(&"L1500"),
        "{:?}",
        &lines[..lines.len().min(5)]
    );
    assert!(!lines.contains(&"L11000"), "still in scrollback");
    let newest = history(&mut client, block, 1, 0);
    assert_eq!(Some(newest.as_str()), lines.last().copied());
    let paged = history(&mut client, block, 2, 1);
    assert_eq!(
        paged.lines().collect::<Vec<_>>(),
        lines[lines.len() - 3..lines.len() - 1]
    );
    drop(client);
    server.stop();

    let server = Server::start(&dir);
    let mut client = Client::connect_with(&server.socket, features, None);
    // The restart marker scrolls a few more lines out of the scrollback.
    let after = history(&mut client, block, 100_000, 0);
    assert!(
        after.starts_with(&all) && after.len() > all.len(),
        "kept, then the lines the marker pushed out"
    );
    client.request(|req| Message::CloseBlock { req, block });
    gone(&dir.join(format!("{block}.spool")));
    drop(client);
    server.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Waits for `path` to be deleted.
#[cfg(unix)]
fn gone(path: &Path) {
    let deadline = std::time::Instant::now() + PATIENCE;
    while path.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "{} not removed",
            path.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// A block's spool goes when its program ends by itself, shell or command.
#[cfg(unix)]
#[test]
fn a_spool_goes_when_its_program_exits() {
    let dir = state_dir("spool-exit");
    let server = Server::start(&dir);
    let mut client = Client::connect_with(
        &server.socket,
        Features::CHECKPOINT_V6.union(Features::LAUNCH),
        None,
    );
    let shell = block_for(&mut client);
    let shell_spool = dir.join(format!("{shell}.spool"));
    assert!(shell_spool.is_dir(), "opened with the block");
    client.input(shell, b"exit\r");
    gone(&shell_spool);
    // Holding the command back until its spool is seen.
    let script = "read go; seq 12000";
    let command = command_block(&mut client, script);
    let command_spool = dir.join(format!("{command}.spool"));
    assert!(command_spool.is_dir(), "opened with the block");
    client.request(|req| Message::AcquireLease {
        req,
        block: command,
    });
    client.input(command, b"go\r");
    gone(&command_spool);
    drop(client);
    server.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A spool that failed to write is dropped, and its files still go with
/// the block.
#[cfg(unix)]
#[test]
fn a_failed_spool_still_goes_with_its_block() {
    let dir = state_dir("spool-failed");
    let features = Features::CHECKPOINT_V6
        .union(Features::BLOCK_CALL)
        .union(Features::TERMINAL_HISTORY);
    let server = Server::start(&dir);
    let mut client = Client::connect_with(&server.socket, features, None);
    let block = block_for(&mut client);
    let spool = dir.join(format!("{block}.spool"));
    // Its first segment cannot be opened for writing.
    std::fs::create_dir(spool.join("0000000000.seg")).unwrap();
    client.input(block, b"seq 12000; echo DONE$((6*7))\r");
    client.wait_for(block, "DONE42");
    let refused = client.answer(|req| Message::BlockCall {
        req,
        block,
        method: "terminal.history".into(),
        args: vec![(
            "history".to_owned(),
            rusty_mux_proto::schema::Value::Lines(10),
        )],
    });
    assert!(
        matches!(refused.as_slice(), [Message::Failed { reason, .. }] if reason.contains("no history")),
        "{refused:?}"
    );
    assert!(spool.is_dir(), "left by the failure");
    client.request(|req| Message::CloseBlock { req, block });
    gone(&spool);
    drop(client);
    server.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

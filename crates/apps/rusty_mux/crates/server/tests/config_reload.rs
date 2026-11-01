//! The config file's `server` section and `reload-config` (C38): what a new
//! block runs and where, read at start and again on request.

#![cfg(target_os = "linux")]

mod common;

use std::path::{Path, PathBuf};

use common::{Client, new_block, shell, start_server, start_server_with, window_for};
use rusty_mux_core::{BlockId, Event, WindowId};
use rusty_mux_proto::{Features, Message};
use rusty_mux_server::{Config, serve};
use rusty_tokio::io::UnixListener;
use rusty_tokio::{Builder, LocalOptions};

const PLAIN: Features = Features::CHECKPOINT_V6
    .union(Features::EVENT_SEQ)
    .union(Features::INSPECT);
const RELOAD: Features = PLAIN.union(Features::CONFIG_RELOAD);

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rusty-mux-reload-{}-{name}", std::process::id()))
}

/// A fresh directory, as `pwd -P` prints it.
fn dir(name: &str) -> PathBuf {
    let path = temp(name);
    std::fs::create_dir_all(&path).expect("mkdir");
    path.canonicalize().expect("canonical")
}

fn write_cwd(file: &Path, cwd: &Path) {
    let text = format!(r#"{{"server": {{"cwd": "{}"}}}}"#, cwd.display());
    std::fs::write(file, text).expect("write config");
}

fn with_file(file: &Path) -> Config {
    Config {
        config_file: Some(file.to_owned()),
        ..shell()
    }
}

/// A new attached, leased block of `window`, and the directory it started in.
fn started_in(client: &mut Client, window: WindowId) -> String {
    let block: BlockId = new_block(client, window);
    client.request(|req| Message::Attach { req, block });
    client.request(|req| Message::AcquireLease { req, block });
    client.input(block, b"echo DIR=$(pwd -P)=\r");
    // The echo shows `$(pwd -P)`; only the output has a path in it.
    client.wait_for(block, "DIR=/");
    let text = String::from_utf8_lossy(client.output(block)).into_owned();
    let start = text.find("DIR=/").expect("output") + 4;
    let end = start + text[start..].find('=').expect("end");
    text[start..end].to_owned()
}

fn failed(answer: &[Message]) -> &str {
    match answer {
        [Message::Failed { reason, .. }] => reason,
        other => panic!("expected Failed, got {other:?}"),
    }
}

fn reloads(client: &Client) -> usize {
    client
        .events
        .iter()
        .filter(|(_, e)| *e == Event::ConfigReloaded)
        .count()
}

#[test]
fn a_reload_moves_the_next_block_and_a_bad_file_keeps_the_last() {
    let (a, b) = (dir("a"), dir("b"));
    let file = temp("good.json");
    write_cwd(&file, &a);
    let path = start_server_with(with_file(&file));
    let mut client = Client::connect_with(&path, RELOAD, None);
    let mut plain = Client::connect_with(&path, PLAIN, None);
    let window = window_for(&mut client);
    assert_eq!(started_in(&mut client, window), a.display().to_string());

    write_cwd(&file, &b);
    client.request(|req| Message::ReloadConfig { req });
    while reloads(&client) == 0 {
        client.recv();
    }
    assert_eq!(started_in(&mut client, window), b.display().to_string());

    std::fs::write(&file, r#"{"server": {"cwd": "relative", "shell": 1}}"#).unwrap();
    let answer = client.answer(|req| Message::ReloadConfig { req });
    let reason = failed(&answer);
    assert!(
        reason.contains("server.cwd") && reason.contains("`shell`"),
        "{reason}"
    );
    assert_eq!(started_in(&mut client, window), b.display().to_string());
    assert_eq!(reloads(&client), 1, "a refused reload announces nothing");

    // Without the feature: refused, and never told.
    let answer = plain.answer(|req| Message::ReloadConfig { req });
    assert!(failed(&answer).contains("feature"), "{answer:?}");
    plain.request(|req| Message::List { req });
    assert_eq!(reloads(&plain), 0, "{:?}", plain.events);
}

#[test]
fn a_server_without_a_file_refuses_a_reload() {
    let path = start_server();
    let mut client = Client::connect_with(&path, RELOAD, None);
    let answer = client.answer(|req| Message::ReloadConfig { req });
    assert!(failed(&answer).contains("no config file"), "{answer:?}");
}

#[test]
fn a_bad_file_stops_the_server_before_it_serves() {
    let file = temp("bad.json");
    std::fs::write(&file, r#"{"server": {"command": ""}}"#).unwrap();
    let socket = temp("bad.sock");
    let runtime = Builder::new_current_thread()
        .build_local(&LocalOptions::default())
        .expect("runtime");
    let err = runtime
        .block_on(async {
            let listener = UnixListener::bind(&socket)?;
            serve(listener, with_file(&file)).await
        })
        .expect_err("refused");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("server.command"), "{err}");
}

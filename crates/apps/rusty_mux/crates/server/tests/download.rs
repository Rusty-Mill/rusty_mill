//! `terminal.download` (C83): a file of the server's host read a chunk at a
//! time by the block's input-lease holder.

#![cfg(target_os = "linux")]

mod common;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use common::{Client, block_for, shell, start_server_with, window_for};
use rusty_mux_core::{BlockId, Size};
use rusty_mux_proto::schema::Value;
use rusty_mux_proto::{Features, Launch, Message};
use rusty_mux_server::Config;

const DOWNLOAD: Features = Features::CHECKPOINT_V6
    .union(Features::INSPECT)
    .union(Features::BLOCK_CALL)
    .union(Features::DOWNLOAD);

fn scratch() -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rusty-mux-download-it-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn read(client: &mut Client, block: BlockId, path: &str, offset: u32) -> Result<String, String> {
    let args = vec![
        ("path".to_owned(), Value::Text(path.to_owned())),
        ("offset".to_owned(), Value::Lines(offset)),
    ];
    let mut answer = client.answer(move |req| Message::BlockCall {
        req,
        block,
        method: "terminal.download".into(),
        args,
    });
    match answer.remove(0) {
        Message::Text { text, .. } => Ok(text),
        Message::Failed { reason, .. } => Err(reason),
        other => panic!("expected Text or Failed, got {other:?}"),
    }
}

fn serve(state: Option<PathBuf>) -> PathBuf {
    start_server_with(Config {
        state_dir: state,
        protected: Default::default(),
        ..shell()
    })
}

#[test]
fn a_file_is_read_with_its_size() {
    let dir = scratch();
    let file = dir.join("f.txt");
    fs::write(&file, b"hello").unwrap();
    let mut client = Client::connect_with(&serve(None), DOWNLOAD, None);
    let block = block_for(&mut client);
    assert_eq!(
        read(&mut client, block, file.to_str().unwrap(), 0),
        Ok("5\naGVsbG8=".into())
    );
    assert_eq!(
        read(&mut client, block, file.to_str().unwrap(), 3),
        Ok("5\nbG8=".into())
    );
    assert_eq!(
        read(&mut client, block, file.to_str().unwrap(), 9),
        Ok("5\n".into())
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_directory_and_the_state_directory_are_refused() {
    let (dir, state) = (scratch(), scratch());
    fs::write(state.join("secret"), b"x").unwrap();
    let mut client = Client::connect_with(&serve(Some(state.clone())), DOWNLOAD, None);
    let block = block_for(&mut client);
    let err = read(&mut client, block, dir.to_str().unwrap(), 0).unwrap_err();
    assert!(err.contains("not a regular file"), "{err}");
    let err = read(
        &mut client,
        block,
        state.join("secret").to_str().unwrap(),
        0,
    )
    .unwrap_err();
    assert!(err.contains("state directory"), "{err}");
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(state);
}

#[test]
fn the_state_directory_stays_out_of_reach_when_its_store_cannot_open() {
    // The configured path is a file, so no store opens there; it is still
    // the server's state directory, and its contents are still not read.
    let (dir, tmp) = (scratch(), scratch());
    fs::write(dir.join("f"), b"ok").unwrap();
    let state = tmp.join("not-a-dir");
    fs::write(&state, b"secret").unwrap();
    let mut client = Client::connect_with(&serve(Some(state.clone())), DOWNLOAD, None);
    let block = block_for(&mut client);
    let err = read(&mut client, block, state.to_str().unwrap(), 0).unwrap_err();
    assert!(err.contains("state directory"), "{err}");
    // A path under a file cannot be resolved: nothing is read at all.
    let under = tmp.join("not-a-dir").join("sub");
    let mut client = Client::connect_with(&serve(Some(under)), DOWNLOAD, None);
    let block = block_for(&mut client);
    let err = read(&mut client, block, dir.join("f").to_str().unwrap(), 0).unwrap_err();
    assert!(err.contains("cannot be resolved"), "{err}");
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(tmp);
}

#[test]
fn only_the_lease_holder_may_download() {
    let dir = scratch();
    fs::write(dir.join("f"), b"x").unwrap();
    let path = serve(None);
    let mut owner = Client::connect_with(&path, DOWNLOAD, None);
    let block = block_for(&mut owner);
    let mut viewer = Client::connect_with(&path, DOWNLOAD, None);
    viewer.request(|req| Message::Attach { req, block });
    let err = read(&mut viewer, block, dir.join("f").to_str().unwrap(), 0).unwrap_err();
    assert!(err.contains("lease"), "{err}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn an_agents_block_refuses_a_download() {
    let path = serve(None);
    let mut client = Client::connect_with(
        &path,
        DOWNLOAD
            .union(Features::LAUNCH)
            .union(Features::AGENT_LAUNCH),
        None,
    );
    let window = window_for(&mut client);
    // Launched as an agent's block is: with an exact environment, and
    // without saying it is one.
    let launch = Launch {
        env: Some(vec![("PATH".into(), "/usr/bin:/bin".into())]),
        ..Launch::default()
    };
    let size = Size::new(80, 24).expect("size");
    let id = client.request(|req| Message::CreateBlock {
        req,
        window,
        size,
        launch,
    });
    let block = BlockId::from_uuid(id.expect("a block id"));
    client.request(|req| Message::Attach { req, block });
    client.request(|req| Message::AcquireLease { req, block });
    let err = read(&mut client, block, "/etc/hostname", 0).unwrap_err();
    assert!(err.contains("agent"), "{err}");
}

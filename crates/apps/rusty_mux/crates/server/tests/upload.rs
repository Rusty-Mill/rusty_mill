//! `terminal.upload` (C82): a file sent to a block's folder under the state
//! directory, a chunk at a time, by the block's input-lease holder.

#![cfg(target_os = "linux")]

mod common;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use common::{Client, block_for, shell, start_server_with};
use rusty_mux_core::BlockId;
use rusty_mux_proto::schema::Value;
use rusty_mux_proto::{Features, Message};
use rusty_mux_server::Config;

const UPLOAD: Features = Features::CHECKPOINT_V6
    .union(Features::INSPECT)
    .union(Features::BLOCK_CALL)
    .union(Features::UPLOAD);

fn state_dir() -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "rusty-mux-upload-it-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn serve(state: Option<PathBuf>) -> PathBuf {
    start_server_with(Config {
        state_dir: state,
        protected: Vec::new(),
        ..shell()
    })
}

/// One chunk; the reply's text, or the refusal's reason.
fn chunk(
    client: &mut Client,
    block: BlockId,
    name: &str,
    offset: u32,
    data: &[u8],
    done: bool,
) -> Result<String, String> {
    let mut args = vec![
        ("name".to_owned(), Value::Text(name.to_owned())),
        ("offset".to_owned(), Value::Lines(offset)),
        ("data".to_owned(), Value::Bytes(data.to_vec())),
    ];
    if done {
        args.push(("done".to_owned(), Value::Flag(true)));
    }
    let mut answer = client.answer(move |req| Message::BlockCall {
        req,
        block,
        method: "terminal.upload".into(),
        args,
    });
    match answer.remove(0) {
        Message::Text { text, .. } => Ok(text),
        Message::Failed { reason, .. } => Err(reason),
        other => panic!("expected Text or Failed, got {other:?}"),
    }
}

#[test]
fn chunks_arrive_as_one_file_and_the_reply_is_its_path() {
    let state = state_dir();
    let path = serve(Some(state.clone()));
    let mut client = Client::connect_with(&path, UPLOAD, None);
    let block = block_for(&mut client);

    assert_eq!(
        chunk(&mut client, block, "../a b.txt", 0, b"hello ", false),
        Ok("6".into())
    );
    let at = chunk(&mut client, block, "../a b.txt", 6, b"world", true).unwrap();
    let at = PathBuf::from(at);
    assert!(at.is_absolute() && at.starts_with(&state), "{at:?}");
    assert_eq!(at.file_name().unwrap(), "a b.txt");
    assert_eq!(fs::read(&at).unwrap(), b"hello world");

    // The block's folder goes with the block.
    client.request(|req| Message::CloseBlock { req, block });
    let folder = at.parent().unwrap().to_path_buf();
    for _ in 0..100 {
        if !folder.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(!folder.exists(), "{folder:?} outlived its block");
    let _ = fs::remove_dir_all(state);
}

#[test]
fn a_chunk_out_of_place_is_refused() {
    let state = state_dir();
    let path = serve(Some(state.clone()));
    let mut client = Client::connect_with(&path, UPLOAD, None);
    let block = block_for(&mut client);
    chunk(&mut client, block, "f", 0, b"abc", false).unwrap();
    let err = chunk(&mut client, block, "f", 1, b"x", true).unwrap_err();
    assert!(err.contains("3 bytes were received"), "{err}");
    let err = chunk(&mut client, block, "///", 0, b"x", true).unwrap_err();
    assert!(err.contains("name"), "{err}");
    let _ = fs::remove_dir_all(state);
}

#[test]
fn only_the_lease_holder_may_upload() {
    let state = state_dir();
    let path = serve(Some(state.clone()));
    let mut owner = Client::connect_with(&path, UPLOAD, None);
    let block = block_for(&mut owner);
    let mut viewer = Client::connect_with(&path, UPLOAD, None);
    viewer.request(|req| Message::Attach { req, block });
    let err = chunk(&mut viewer, block, "f", 0, b"x", true).unwrap_err();
    assert!(err.contains("lease"), "{err}");
    let _ = fs::remove_dir_all(state);
}

#[test]
fn a_server_without_a_state_directory_refuses() {
    let path = serve(None);
    let mut client = Client::connect_with(&path, UPLOAD, None);
    let block = block_for(&mut client);
    let err = chunk(&mut client, block, "f", 0, b"x", true).unwrap_err();
    assert!(err.contains("state directory"), "{err}");
}

#[test]
fn a_client_without_the_bit_is_refused() {
    let state = state_dir();
    let path = serve(Some(state.clone()));
    let mut client = Client::connect_with(
        &path,
        Features::from_bits(UPLOAD.bits() & !Features::UPLOAD.bits()),
        None,
    );
    let block = block_for(&mut client);
    let err = chunk(&mut client, block, "f", 0, b"x", true).unwrap_err();
    assert!(err.contains("upload") || err.contains("feature"), "{err}");
    let _ = fs::remove_dir_all(state);
}

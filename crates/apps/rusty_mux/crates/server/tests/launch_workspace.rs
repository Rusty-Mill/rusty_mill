//! A launch in another block's workspace (D378, card #1573): the new block
//! starts in the directory the server holds for that block, by handle, and
//! the launch is refused wherever that cannot be said truthfully. Linux only:
//! the held directory is entered through `/proc`.

#![cfg(target_os = "linux")]

mod common;

use common::{Client, shell, start_server_with, window_for};
use rusty_mux_core::{BlockId, Size, WindowId};
use rusty_mux_proto::{Features, Launch, Message, Program};
use std::path::PathBuf;

const BASE: Features = Features::CHECKPOINT_V6
    .union(Features::LAUNCH)
    .union(Features::AGENT_LAUNCH)
    .union(Features::WORKSPACE);
const WITH_BIT: Features = BASE.union(Features::LAUNCH_IN_WORKSPACE);

/// A fresh directory, real path.
fn dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("wsof-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("dir");
    path.canonicalize().expect("real path")
}

/// `/bin/sh -c`: waits for a line, prints its directory, then sleeps.
fn pwd() -> Option<Program> {
    Some(Program {
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "read go; pwd -P; sleep 30".into()],
    })
}

fn create(window: WindowId, launch: Launch) -> impl FnOnce(u64) -> Message {
    let size = Size::new(200, 24).expect("size");
    move |req| Message::CreateBlock {
        req,
        window,
        size,
        launch,
    }
}

fn launched(answer: &[Message]) -> Result<BlockId, String> {
    match answer {
        [
            ..,
            Message::Done {
                created: Some(id), ..
            },
        ] => Ok(BlockId::from_uuid(*id)),
        [.., Message::Failed { reason, .. }] => Err(reason.clone()),
        other => panic!("unexpected answer {other:?}"),
    }
}

fn lease(client: &mut Client, block: BlockId) {
    client.request(|req| Message::Attach { req, block });
    client.request(|req| Message::AcquireLease { req, block });
}

/// A server whose configured directory is `home`, a client with `features`,
/// and a leased block launched in `work`.
fn setup(
    features: Features,
    home: &PathBuf,
    work: &PathBuf,
) -> (PathBuf, Client, WindowId, BlockId) {
    let socket = start_server_with(rusty_mux_server::Config {
        cwd: home.clone().into_os_string(),
        ..shell()
    });
    let mut client = Client::connect_with(&socket, features, None);
    let window = window_for(&mut client);
    let source = Launch {
        cwd: Some(work.to_string_lossy().into_owned()),
        ..Launch::default()
    };
    let block = launched(&client.answer(create(window, source))).expect("source block");
    lease(&mut client, block);
    (socket, client, window, block)
}

fn in_workspace(source: BlockId) -> Launch {
    Launch {
        program: pwd(),
        workspace_of: Some(source),
        ..Launch::default()
    }
}

#[test]
fn the_new_block_starts_in_the_held_directory_even_after_a_rename() {
    let (home, work) = (dir("home"), dir("work"));
    let (_socket, mut client, window, source) = setup(WITH_BIT, &home, &work);
    let moved = work.with_file_name(format!("wsof-{}-moved", std::process::id()));
    let _ = std::fs::remove_dir_all(&moved);
    std::fs::rename(&work, &moved).expect("rename");
    let block = launched(&client.answer(create(window, in_workspace(source)))).expect("launch");
    lease(&mut client, block);
    client.input(block, b"\r");
    // The directory it holds, wherever the path went; not the configured one.
    client.wait_for(block, moved.to_str().expect("utf8"));
    assert!(!client.saw(block, home.to_str().expect("utf8")));
}

#[test]
fn it_is_refused_without_the_bit_and_for_both_cwd_and_workspace() {
    let (home, work) = (dir("home2"), dir("work2"));
    let (_socket, mut client, window, source) = setup(BASE, &home, &work);
    let reason = launched(&client.answer(create(window, in_workspace(source)))).unwrap_err();
    assert!(reason.contains("requires feature bits"), "{reason}");
    let (_socket, mut client, window, source) = setup(WITH_BIT, &home, &work);
    let both = Launch {
        cwd: Some(home.to_string_lossy().into_owned()),
        ..in_workspace(source)
    };
    let reason = launched(&client.answer(create(window, both))).unwrap_err();
    assert!(reason.contains("bad_launch(workspace_of)"), "{reason}");
}

#[test]
fn it_is_refused_for_an_unknown_block_and_without_the_lease() {
    let (home, work) = (dir("home3"), dir("work3"));
    let (socket, mut client, window, source) = setup(WITH_BIT, &home, &work);
    let unknown = BlockId::from_uuid(rusty_uuid::Uuid::new_v4());
    assert!(launched(&client.answer(create(window, in_workspace(unknown)))).is_err());
    // Another client holds no lease on the source block.
    let mut other = Client::connect_with(&socket, WITH_BIT, None);
    let window = window_for(&mut other);
    let reason = launched(&other.answer(create(window, in_workspace(source)))).unwrap_err();
    assert!(!reason.is_empty(), "{reason}");
}

#[test]
fn it_is_refused_on_an_agents_block() {
    let (home, work) = (dir("home4"), dir("work4"));
    let (_socket, mut client, window, _) = setup(WITH_BIT, &home, &work);
    // Launched as an agent's block is: with an exact environment.
    let agent = Launch {
        env: Some(vec![("PATH".into(), "/usr/bin:/bin".into())]),
        cwd: Some(work.to_string_lossy().into_owned()),
        ..Launch::default()
    };
    let block = launched(&client.answer(create(window, agent))).expect("agent block");
    lease(&mut client, block);
    let reason = launched(&client.answer(create(window, in_workspace(block)))).unwrap_err();
    assert!(reason.contains("agent"), "{reason}");
}

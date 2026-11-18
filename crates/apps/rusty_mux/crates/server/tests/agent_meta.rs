//! Agent, role and card on a block (D336): the server stores and echoes
//! them, lists them only to a client that asked, and refuses a launch that
//! names them without the feature. Unix only: the program is `/bin/sh`.

#![cfg(unix)]

mod common;

use common::{Client, start_server, window_for};
use rusty_mux_core::{BlockId, Size};
use rusty_mux_proto::{AgentMeta, Entry, Features, Launch, Message, Program};

const BASE: Features = Features::CHECKPOINT_V6
    .union(Features::LAUNCH)
    .union(Features::INSPECT);

fn meta() -> AgentMeta {
    AgentMeta {
        agent: "claude".into(),
        role: "builder".into(),
        card: "#1572".into(),
    }
}

fn create(client: &mut Client, agent: Option<AgentMeta>) -> Vec<Message> {
    let window = window_for(client);
    let launch = Launch {
        program: Some(Program {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 30".into()],
        }),
        agent,
        ..Launch::default()
    };
    client.answer(|req| Message::CreateBlock {
        req,
        window,
        size: Size::new(80, 24).expect("size"),
        launch,
    })
}

fn created(answer: &[Message]) -> BlockId {
    match answer {
        [
            ..,
            Message::Done {
                created: Some(id), ..
            },
        ] => BlockId::from_uuid(*id),
        other => panic!("not created: {other:?}"),
    }
}

fn listed(client: &mut Client, block: BlockId) -> Option<AgentMeta> {
    client
        .answer(|req| Message::List { req })
        .into_iter()
        .find_map(|m| match m {
            Message::Entry {
                entry: Entry::Agent { block: b, meta },
                ..
            } if b == block => Some(meta),
            _ => None,
        })
}

#[test]
fn a_launch_names_its_agent_and_the_listing_echoes_it() {
    let socket = start_server();
    let mut client = Client::connect_with(&socket, BASE.union(Features::AGENT_META), None);
    let block = created(&create(&mut client, Some(meta())));
    assert_eq!(listed(&mut client, block), Some(meta()));
}

#[test]
fn a_block_launched_without_an_agent_lists_no_agent_entry() {
    let socket = start_server();
    let mut client = Client::connect_with(&socket, BASE.union(Features::AGENT_META), None);
    let block = created(&create(&mut client, None));
    assert_eq!(listed(&mut client, block), None);
}

#[test]
fn a_client_that_did_not_ask_is_not_sent_the_entry() {
    let socket = start_server();
    let mut owner = Client::connect_with(&socket, BASE.union(Features::AGENT_META), None);
    let block = created(&create(&mut owner, Some(meta())));
    let mut other = Client::connect_with(&socket, BASE, None);
    assert_eq!(listed(&mut other, block), None);
}

#[test]
fn a_launch_naming_an_agent_needs_the_feature() {
    let socket = start_server();
    let mut client = Client::connect_with(&socket, BASE, None);
    let answer = create(&mut client, Some(meta()));
    assert!(
        matches!(answer.as_slice(), [Message::Failed { reason, .. }] if reason.contains("requires feature bits")),
        "{answer:?}"
    );
}

use rusty_mux_core::{BlockId, ClientId, SessionId, Size, WindowId};

use super::*;

fn id(text: &str) -> Uuid {
    text.parse().expect("uuid")
}

const S1: &str = "00000000-0000-0000-0000-00000000aa01";
const S2: &str = "00000000-0000-0000-0000-00000000bb02";
const W1: &str = "00000000-0000-0000-0000-00000000cc03";
const W2: &str = "00000000-0000-0000-0000-00000000dd04";
const B1: &str = "00000000-0000-0000-0000-00000000ee05";
const B2: &str = "00000000-0000-0000-0000-00000000ff06";
const B3: &str = "00000000-0000-0000-0000-00000000ff07";
const C1: &str = "00000000-0000-0000-0000-00000000c108";
const C2: &str = "00000000-0000-0000-0000-00000000c209";

/// Session "build" holds window "logs" with blocks B1 and B2, named
/// "tests"; session "Main" holds a window also named "logs" with block B3.
/// Client C1 is unnamed, C2 is "Laptop".
fn listing() -> Vec<Entry> {
    let session = |s: &str, name: &str| Entry::Session {
        session: SessionId::from_uuid(id(s)),
        name: name.into(),
    };
    let window = |w: &str, s: &str| Entry::Window {
        window: WindowId::from_uuid(id(w)),
        session: SessionId::from_uuid(id(s)),
        name: "logs".into(),
        layout: None,
        zoomed: None,
    };
    let block = |b: &str, w: &str, name: &str| Entry::Block {
        block: BlockId::from_uuid(id(b)),
        window: WindowId::from_uuid(id(w)),
        size: Size::new(80, 24).expect("size"),
        holder: None,
        title: String::new(),
        name: name.into(),
    };
    let client = |c: &str, name: &str| Entry::Client {
        client: ClientId::from_uuid(id(c)),
        name: name.into(),
        attached: None,
    };
    vec![
        session(S1, "build"),
        window(W1, S1),
        block(B1, W1, ""),
        block(B2, W1, "tests"),
        session(S2, "Main"),
        window(W2, S2),
        block(B3, W2, ""),
        client(C1, ""),
        client(C2, "Laptop"),
    ]
}

fn find(entity: Entity, spec: &str, session: Option<&str>) -> Result<Uuid, Miss> {
    resolve(&listing(), entity, spec, session.map(id))
}

#[test]
fn rules_apply_in_order() {
    assert_eq!(find(Entity::Session, S2, None), Ok(id(S2)));
    assert_eq!(find(Entity::Session, "main", None), Ok(id(S2)));
    assert_eq!(find(Entity::Session, "2", None), Ok(id(S2)));
    assert_eq!(find(Entity::Session, "bb02", None), Ok(id(S2)));
    assert_eq!(find(Entity::Block, "EE05", None), Ok(id(B1)));
}

#[test]
fn paths_and_the_default_session_narrow_the_search() {
    assert_eq!(find(Entity::Window, "main/logs", None), Ok(id(W2)));
    assert_eq!(find(Entity::Block, "build/1/2", None), Ok(id(B2)));
    assert_eq!(find(Entity::Block, "logs/1", Some(S2)), Ok(id(B3)));
    assert_eq!(find(Entity::Block, "2", Some(S1)), Ok(id(B2)));
    // A session part replaces the default.
    assert_eq!(find(Entity::Block, "main/1/1", Some(S1)), Ok(id(B3)));
}

#[test]
fn several_matches_are_ambiguous() {
    assert_eq!(
        find(Entity::Window, "logs", None),
        Err(Miss::Ambiguous(
            "window `logs`".into(),
            vec![id(W1), id(W2)]
        ))
    );
}

#[test]
fn misses_name_what_was_looked_for() {
    assert_eq!(
        find(Entity::Session, "nope", None),
        Err(Miss::NotFound("no session `nope`".into()))
    );
    for (entity, spec) in [
        (Entity::Session, "nope"),
        (Entity::Session, "3"),
        (Entity::Session, "0"),
        (Entity::Window, "build/2"),
        (Entity::Session, "a/b"),
        (Entity::Block, ""),
        // A suffix needs four characters, and must end the id.
        (Entity::Block, "f06"),
        (Entity::Block, "0000000ff0"),
    ] {
        assert!(
            matches!(find(entity, spec, None), Err(Miss::NotFound(_))),
            "{spec}"
        );
    }
}

#[test]
fn blocks_and_clients_are_named_like_the_rest() {
    assert_eq!(find(Entity::Block, "TESTS", None), Ok(id(B2)));
    assert_eq!(find(Entity::Block, "build/logs/tests", None), Ok(id(B2)));
    assert_eq!(find(Entity::Client, "laptop", None), Ok(id(C2)));
    assert_eq!(find(Entity::Client, "1", Some(S1)), Ok(id(C1)));
    assert_eq!(find(Entity::Client, "c108", None), Ok(id(C1)));
    for spec in ["", "x/laptop", "logs"] {
        assert!(
            matches!(find(Entity::Client, spec, None), Err(Miss::NotFound(_))),
            "{spec}"
        );
    }
}

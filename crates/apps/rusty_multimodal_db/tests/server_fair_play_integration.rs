//! `FPL-FR-007` (ADR-0137): the three `Fair Play` tables — `card`
//! (primary), `person`, `card_default` — over `serve_tables` (ADR-0050),
//! driven by the Rust `SchemaDrivenClient` and by the Python reference
//! client (`clients/python/fair_play_driver.py`). The data directory is
//! reopened by a second server to prove a reassignment survived.
//!
//! **The Python half needs a CPython 3 interpreter on `PATH` and fails
//! loudly without one** — the same `python3`-then-`python` lookup and
//! the same named panic `tests/server_python_client.rs` uses; a skipped
//! test is not a test.

#![cfg(feature = "server")]

use rusty_multimodal_db::generic::fair_play::{
    card_default_id, deck_card_id, insert_card, open_or_create_card_default_production_stack,
    open_or_create_card_production_stack, open_or_create_person_production_stack, person_id,
    split_card, split_card_id, CardDefault, Person, SplitSpec, Suit, CARD_DEFAULT_FILE, CARD_FILE,
    PERSON_FILE,
};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::client::{BatchOp, ClientError, SchemaDrivenClient};
use rusty_multimodal_db::server::fair_play::{
    CardConnectionStore, CardDefaultConnectionStore, PersonConnectionStore,
};
use rusty_multimodal_db::server::protocol::{ErrorCode, ParentLookup, ScanValue};
use rusty_multimodal_db::server::{serve_tables, ConnectionStore, ServeOptions};
use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener};
use std::process::Command;
use std::sync::Arc;
use std::thread;
use uuid::Uuid;

fn unique_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{label}_{}_{n}", std::process::id()))
}

/// Four deck cards — 1, 2 Home; 3 Out; 4 Wild — and two people, built
/// inline (the library's fixtures are crate-private).
fn deck() -> Vec<CardDefault> {
    [
        (1, Suit::Home),
        (2, Suit::Home),
        (3, Suit::Out),
        (4, Suit::Wild),
    ]
    .into_iter()
    .map(|(n, suit)| CardDefault {
        id: card_default_id(n),
        number: n,
        name: format!("Card {n}"),
        suit,
        conception: format!("conceive {n}"),
        planning: format!("plan {n}"),
        execution: format!("execute {n}"),
        minimum_standard_of_care: vec![format!("std {n} a"), format!("std {n} b")],
    })
    .collect()
}

fn people() -> Vec<Person> {
    vec![
        Person {
            id: person_id("Ada"),
            name: "Ada".into(),
            player: 1,
        },
        Person {
            id: person_id("Bob"),
            name: "Bob".into(),
            player: 2,
        },
    ]
}

fn spec(path: &str, name: &str, owner: Option<Uuid>) -> SplitSpec {
    SplitSpec {
        id: split_card_id(path),
        name: name.into(),
        owner_id: owner,
        minimum_standard_of_care: vec![format!("{name} done")],
        ..SplitSpec::default()
    }
}

/// Open the three stacks in `dir` — seeding them, and splitting card 1
/// into `1/a` (unowned) and `1/b` (Bob's), only when the directory is
/// new — and serve them the way `fair_play_server` does: `card` primary.
fn start_server_at(dir: std::path::PathBuf) -> SocketAddr {
    std::fs::create_dir_all(&dir).unwrap();
    let fresh = !dir.join(CARD_FILE).exists();
    let mut people_stack = open_or_create_person_production_stack(&dir.join(PERSON_FILE)).unwrap();
    let mut card_stack = open_or_create_card_production_stack(&dir.join(CARD_FILE)).unwrap();
    let mut default_stack =
        open_or_create_card_default_production_stack(&dir.join(CARD_DEFAULT_FILE)).unwrap();
    if fresh {
        for person in people() {
            people_stack.insert(person).unwrap();
        }
        for default in deck() {
            insert_card(&mut card_stack, default.to_card()).unwrap();
            default_stack.insert(default).unwrap();
        }
        split_card(
            &mut card_stack,
            deck_card_id(1),
            vec![
                spec("1/a", "a", None),
                spec("1/b", "b", Some(person_id("Bob"))),
            ],
            None,
        )
        .unwrap();
    }
    let card: Arc<dyn ConnectionStore> = Arc::new(CardConnectionStore::new(
        GenericProductionStore::new(card_stack),
    ));
    let person: Arc<dyn ConnectionStore> = Arc::new(PersonConnectionStore::new(
        GenericProductionStore::new(people_stack),
    ));
    let card_default: Arc<dyn ConnectionStore> = Arc::new(CardDefaultConnectionStore::new(
        GenericProductionStore::new(default_stack),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        serve_tables(
            listener,
            vec![
                ("card".to_string(), card),
                ("person".to_string(), person),
                ("card_default".to_string(), card_default),
            ],
            0,
            ServeOptions::default(),
        )
    });
    addr
}

fn value<'a>(fields: &'a [(String, ScanValue)], name: &str) -> &'a ScanValue {
    fields
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v)
        .unwrap_or_else(|| panic!("no field {name} in {fields:?}"))
}

/// `fields` with `name` set to `value`, as a `replace`/`insert` argument.
fn with(fields: &[(String, ScanValue)], name: &str, value: ScanValue) -> Vec<(String, ScanValue)> {
    fields
        .iter()
        .map(|(n, v)| (n.clone(), if n == name { value.clone() } else { v.clone() }))
        .collect()
}

fn borrowed(fields: &[(String, ScanValue)]) -> Vec<(&str, ScanValue)> {
    fields
        .iter()
        .map(|(n, v)| (n.as_str(), v.clone()))
        .collect()
}

#[test]
fn malformed_atomic_batches_leave_card_and_person_records_unchanged() {
    let addr = start_server_at(unique_dir("fair_play_atomic_batches"));
    let mut client = SchemaDrivenClient::connect(addr).unwrap();

    let card_id = deck_card_id(1);
    let card_before = client.get(card_id).unwrap().unwrap();
    let malformed_card = borrowed(&card_before[..card_before.len() - 1]);
    let card_ops = [
        BatchOp::UpdateField {
            id: card_id,
            field: "position",
            value: ScanValue::U32(999),
        },
        BatchOp::Replace {
            id: card_id,
            fields: &malformed_card,
        },
    ];
    assert!(matches!(
        client.write_batch(&card_ops, true),
        Err(ClientError::TransactionFailed {
            index: 0,
            code: ErrorCode::Unsupported,
            ..
        })
    ));
    assert_eq!(client.get(card_id).unwrap().unwrap(), card_before);

    client.use_table("person").unwrap();
    let person = person_id("Ada");
    let person_before = client.get(person).unwrap().unwrap();
    let malformed_person = borrowed(&person_before[..person_before.len() - 1]);
    let person_ops = [
        BatchOp::UpdateField {
            id: person,
            field: "player",
            value: ScanValue::U32(999),
        },
        BatchOp::Replace {
            id: person,
            fields: &malformed_person,
        },
    ];
    assert!(matches!(
        client.write_batch(&person_ops, true),
        Err(ClientError::TransactionFailed {
            index: 0,
            code: ErrorCode::Unsupported,
            ..
        })
    ));
    assert_eq!(client.get(person).unwrap().unwrap(), person_before);
}

#[test]
fn card_insert_enforces_deck_number_uniqueness_under_the_wire_lock() {
    let addr = start_server_at(unique_dir("fair_play_unique_numbers"));
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let template = client.get(deck_card_id(1)).unwrap().unwrap();

    let duplicate_id = Uuid::from_u128(0xd001);
    match client.insert(duplicate_id, &borrowed(&template)) {
        Err(ClientError::Server(ErrorCode::Duplicate, _)) => {}
        other => panic!("expected Duplicate for an existing deck number, got {other:?}"),
    }
    assert!(client.get(duplicate_id).unwrap().is_none());

    let unique = with(&template, "number", ScanValue::U32(98));
    let unique_id = Uuid::from_u128(0xd002);
    client.insert(unique_id, &borrowed(&unique)).unwrap();
    assert_eq!(
        value(&client.get(unique_id).unwrap().unwrap(), "number"),
        &ScanValue::U32(98)
    );

    let family = with(
        &with(
            &with(&template, "number", ScanValue::Null),
            "baseline_id",
            ScanValue::Null,
        ),
        "origin",
        ScanValue::U32(1),
    );
    let family_id = Uuid::from_u128(0xd003);
    client.insert(family_id, &borrowed(&family)).unwrap();
    assert_eq!(
        value(&client.get(family_id).unwrap().unwrap(), "number"),
        &ScanValue::Null
    );

    let concurrent = with(&template, "number", ScanValue::U32(99));
    let attempts: Vec<_> = [Uuid::from_u128(0xd004), Uuid::from_u128(0xd005)]
        .into_iter()
        .map(|id| {
            let fields = concurrent.clone();
            thread::spawn(move || {
                let mut client = SchemaDrivenClient::connect(addr).unwrap();
                (id, client.insert(id, &borrowed(&fields)))
            })
        })
        .collect();
    let outcomes: Vec<_> = attempts.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(
        outcomes
            .iter()
            .filter(|(_, result)| matches!(result, Ok(())))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|(_, result)| matches!(
                result,
                Err(ClientError::Server(ErrorCode::Duplicate, _))
            ))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|(id, _)| client.get(*id).unwrap().is_some())
            .count(),
        1
    );
}

/// Whichever of the two conventional CPython 3 binary names is on
/// `PATH` — `tests/server_python_client.rs::python_binary`, verbatim.
fn python_binary() -> &'static str {
    for candidate in ["python3", "python"] {
        let Ok(output) = Command::new(candidate).arg("--version").output() else {
            continue;
        };
        let version = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if version.contains("Python 3") {
            return candidate;
        }
    }
    panic!(
        "neither python3 nor python (CPython 3) is on PATH — required by \
         tests/server_fair_play_integration.rs (FPL-FR-007, ADR-0137). Install Python 3 or run \
         without this test target."
    )
}

/// Run `clients/python/fair_play_driver.py` once; its `key=value` lines
/// as a map. A missing interpreter is a named panic, never a skip.
fn drive(addr: SocketAddr, card: Uuid) -> HashMap<String, String> {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let output = Command::new(python_binary())
        .arg("clients/python/fair_play_driver.py")
        .arg(addr.to_string())
        .arg(card.to_string())
        .current_dir(manifest)
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "a CPython 3 interpreter is required by tests/server_fair_play_integration.rs \
                 (FPL-FR-007, ADR-0137) and could not be started: {e}. Install Python 3 or run \
                 without this test target."
            )
        });
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "fair_play_driver.py exited with {}\n--- stdout ---\n{stdout}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn the_three_tables_are_served_and_a_reassignment_survives_a_restart() {
    let dir = unique_dir("fair_play_tables");
    let addr = start_server_at(dir.clone());
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let (names, primary) = client.list_tables().unwrap();
    assert_eq!(names, vec!["card", "person", "card_default"]);
    assert_eq!(primary, "card");

    // `person`: the name index.
    client.use_table("person").unwrap();
    assert_eq!(
        client
            .filter_eq("name", ScanValue::Str("Ada".into()))
            .unwrap(),
        vec![person_id("Ada")]
    );
    assert!(client
        .filter_eq("name", ScanValue::Str("Nobody".into()))
        .unwrap()
        .is_empty());
    let ada = client.get(person_id("Ada")).unwrap().unwrap();
    assert_eq!(value(&ada, "player"), &ScanValue::U32(1));

    // `card`: get, the suit index, the tree.
    client.use_table("card").unwrap();
    let one = client.get(deck_card_id(1)).unwrap().unwrap();
    assert_eq!(one.len(), 13);
    assert_eq!(value(&one, "number"), &ScanValue::U32(1));
    assert_eq!(value(&one, "name"), &ScanValue::Str("Card 1".into()));
    assert_eq!(value(&one, "suit"), &ScanValue::U32(0));
    assert_eq!(
        value(&one, "parent_card_id"),
        &ScanValue::Null,
        "the sentinel is Null at this protocol version (ADR-0128)"
    );
    assert_eq!(value(&one, "owner_id"), &ScanValue::Null);
    assert_eq!(
        value(&one, "minimum_standard_of_care"),
        &ScanValue::StrList(vec!["std 1 a".into(), "std 1 b".into()])
    );
    assert_eq!(value(&one, "origin"), &ScanValue::U32(0));
    assert_eq!(
        value(&one, "baseline_id"),
        &ScanValue::Str(card_default_id(1).to_string())
    );
    let mut home = client.filter_eq("suit", ScanValue::U32(0)).unwrap();
    home.sort();
    let mut expected = vec![
        deck_card_id(1),
        deck_card_id(2),
        split_card_id("1/a"),
        split_card_id("1/b"),
    ];
    expected.sort();
    assert_eq!(
        home, expected,
        "the split children copied the parent's suit"
    );
    assert!(matches!(
        client.filter_eq("suit", ScanValue::U32(9)),
        Err(ClientError::Server(ErrorCode::Malformed, _))
    ));
    let mut kids = client.children(deck_card_id(1)).unwrap();
    kids.sort();
    let mut expected = vec![split_card_id("1/a"), split_card_id("1/b")];
    expected.sort();
    assert_eq!(kids, expected);
    assert_eq!(
        client.parent(split_card_id("1/a")).unwrap(),
        ParentLookup::Parent(deck_card_id(1))
    );
    assert_eq!(
        client.parent(deck_card_id(1)).unwrap(),
        ParentLookup::NoParent
    );
    assert!(client.children(deck_card_id(2)).unwrap().is_empty());

    // Reassign `1/a` to Ada with a whole-record replace; the Nulls go
    // back as Nulls and the server stores the sentinels.
    let a = client.get(split_card_id("1/a")).unwrap().unwrap();
    assert_eq!(value(&a, "number"), &ScanValue::Null);
    assert_eq!(value(&a, "owner_id"), &ScanValue::Null);
    assert_eq!(value(&a, "origin"), &ScanValue::U32(1));
    let reassigned = with(&a, "owner_id", ScanValue::Str(person_id("Ada").to_string()));
    assert!(client
        .replace(split_card_id("1/a"), &borrowed(&reassigned))
        .unwrap());
    assert_eq!(
        client.get(split_card_id("1/a")).unwrap().unwrap(),
        reassigned
    );
    assert!(
        !client
            .replace(Uuid::from_u128(4242), &borrowed(&reassigned))
            .unwrap(),
        "an unknown id is false, nothing written"
    );
    // A deck card without a number violates the domain's invariant:
    // refused as Malformed, nothing written.
    let bad = with(
        &with(&one, "number", ScanValue::Null),
        "name",
        ScanValue::Str("x".into()),
    );
    match client.insert(Uuid::from_u128(0xbad), &borrowed(&bad)) {
        Err(ClientError::Server(ErrorCode::Malformed, _)) => {}
        other => panic!("expected Malformed, got {other:?}"),
    }
    assert!(client.get(Uuid::from_u128(0xbad)).unwrap().is_none());
    // Delete is never offered on `card`.
    match client.delete(split_card_id("1/b")) {
        Err(ClientError::Server(ErrorCode::Unsupported, _)) => {}
        other => panic!("expected Unsupported, got {other:?}"),
    }

    // `card_default`: readable, never writable.
    client.use_table("card_default").unwrap();
    let baseline = client.get(card_default_id(1)).unwrap().unwrap();
    assert_eq!(baseline.len(), 7);
    assert_eq!(value(&baseline, "name"), &ScanValue::Str("Card 1".into()));
    assert_eq!(
        client.filter_eq("suit", ScanValue::U32(0)).unwrap().len(),
        2
    );
    match client.insert(Uuid::from_u128(0xbad), &borrowed(&baseline)) {
        Err(ClientError::Server(ErrorCode::Unsupported, _)) => {}
        other => panic!("expected Unsupported, got {other:?}"),
    }
    match client.replace(card_default_id(1), &borrowed(&baseline)) {
        Err(ClientError::Server(ErrorCode::Unsupported, _)) => {}
        other => panic!("expected Unsupported, got {other:?}"),
    }
    drop(client);

    // A second server on the same directory serves the reassignment.
    let addr = start_server_at(dir);
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let a = client.get(split_card_id("1/a")).unwrap().unwrap();
    assert_eq!(
        value(&a, "owner_id"),
        &ScanValue::Str(person_id("Ada").to_string())
    );
    assert_eq!(
        value(&a, "parent_card_id"),
        &ScanValue::Str(deck_card_id(1).to_string())
    );
    assert_eq!(client.children(deck_card_id(1)).unwrap().len(), 2);
    assert!(client.get(Uuid::from_u128(0xbad)).unwrap().is_none());
}

/// The Python reference client reaches `person` and `card` through
/// `use_table`, reads a split deck card with its Nulls, and walks the
/// tree — the same server, a different client.
#[test]
fn the_python_reference_client_reaches_person_and_card() {
    let addr = start_server_at(unique_dir("fair_play_python"));
    let out = drive(addr, deck_card_id(1));
    let get = |k: &str| {
        out.get(k)
            .map(String::as_str)
            .unwrap_or_else(|| panic!("no {k}=: {out:?}"))
    };
    assert_eq!(get("tables"), "card,person,card_default;card");
    assert_eq!(get("person_table"), "person");
    assert_eq!(get("person_fields"), "name,player");
    assert_eq!(get("person_filter_eq_ids"), "1");
    assert_eq!(get("person_filter_eq_first"), person_id("Ada").to_string());
    assert_eq!(get("person_name"), "Ada");
    assert_eq!(get("person_player"), "1");
    assert_eq!(get("card_table"), "card");
    assert_eq!(
        get("card_fields"),
        "number,name,suit,parent_card_id,position,owner_id,conception,planning,execution,\
         minimum_standard_of_care,notes,origin,baseline_id"
    );
    assert_eq!(get("card_get_fields"), "13");
    assert_eq!(get("card_name"), "Card 1");
    assert_eq!(get("card_number"), "1");
    assert_eq!(get("card_owner"), "null", "the sentinel is None in Python");
    assert_eq!(get("card_parent"), "null");
    assert_eq!(get("card_standards"), "std 1 a|std 1 b");
    assert_eq!(get("card_children"), "2");
    let mut ids = [
        split_card_id("1/a").to_string(),
        split_card_id("1/b").to_string(),
    ];
    ids.sort();
    assert_eq!(get("card_children_ids"), ids.join(","));
    assert_eq!(get("card_parent_of_child"), deck_card_id(1).to_string());
    assert_eq!(get("card_get_missing"), "none");
}

//! `ADR-0129` (`QPE-FR-001`..`004`) over a real socket on `Entity`, whose
//! `label` and `kind` are both equality-indexed: a query with an `Eq` on each
//! reads both buckets and decodes only what is in both, and answers exactly
//! what a full scan would.

use rusty_multimodal_db::generic::entity::{create_entity_production_stack, Entity};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::client::{QueryResult, SchemaDrivenClient};
use rusty_multimodal_db::server::entity::EntityConnectionStore;
use rusty_multimodal_db::server::{serve, ServeOptions};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use uuid::Uuid;

const KINDS: [&str; 5] = ["person", "place", "concept", "organization", "event"];

fn start() -> SchemaDrivenClient {
    let dir = std::env::temp_dir().join(format!("planner_eq_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // 500 entities: kind cycles through five values (100 each); labels are
    // unique except that every tenth is "shared", so `label = 'shared'` is
    // itself a 50-record bucket spread across all five kinds.
    let entities: Vec<Entity> = (1..=500u128)
        .map(|n| Entity {
            id: Uuid::from_u128(n),
            label: if n % 10 == 0 {
                "shared".into()
            } else {
                format!("entity {n}")
            },
            kind: KINDS[(n % 5) as usize].into(),
            mention_count: 0,
            aliases: vec![],
        })
        .collect();
    let stack =
        create_entity_production_stack(entities, &[], &[], &dir.join("entities.mmap")).unwrap();
    let store = Arc::new(EntityConnectionStore::new(GenericProductionStore::new(
        stack,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, store, ServeOptions::default()));
    SchemaDrivenClient::connect(addr).unwrap()
}

fn ids(result: QueryResult) -> Vec<Uuid> {
    match result {
        QueryResult::Rows(rows) => {
            let mut ids: Vec<Uuid> = rows.into_iter().map(|(id, _)| id).collect();
            ids.sort();
            ids
        }
        other => panic!("expected Rows, got {other:?}"),
    }
}

/// Both predicates indexed: the set is the records that satisfy both, however
/// the buckets are read.
#[test]
fn two_indexed_equalities_answer_the_full_scans_set() {
    let mut client = start();
    // n % 10 == 0 makes the label "shared"; n % 5 == 0 makes the kind
    // KINDS[0] ("person"): every "shared" entity is a person.
    let shared_people = ids(client
        .query("SELECT label FROM entity WHERE kind = 'person' AND label = 'shared'")
        .unwrap());
    let expected: Vec<Uuid> = (1..=500u128)
        .filter(|n| n % 10 == 0)
        .map(Uuid::from_u128)
        .collect();
    assert_eq!(shared_people, expected);

    // A unique label and the kind it does have, and the kind it does not.
    let one = ids(client
        .query("SELECT label FROM entity WHERE kind = 'place' AND label = 'entity 6'")
        .unwrap());
    assert_eq!(one, vec![Uuid::from_u128(6)]);
    assert!(ids(client
        .query("SELECT label FROM entity WHERE kind = 'person' AND label = 'entity 6'")
        .unwrap())
    .is_empty());
    assert!(ids(client
        .query("SELECT label FROM entity WHERE kind = 'person' AND label = 'no such label'")
        .unwrap())
    .is_empty());
    // Order of the two predicates does not change the answer.
    assert_eq!(
        ids(client
            .query("SELECT label FROM entity WHERE label = 'entity 6' AND kind = 'place'")
            .unwrap()),
        vec![Uuid::from_u128(6)]
    );
    // A third, unindexed predicate is still applied to what is read.
    assert!(ids(client
        .query(
            "SELECT label FROM entity WHERE kind = 'place' AND label = 'entity 6' \
             AND mention_count = 9"
        )
        .unwrap())
    .is_empty());
}

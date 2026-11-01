//! The software factory's workflow (`deploy/factory.workflow.json`, card #584): its columns,
//! its human-only gates, and `card_take` routing each role to its stages.

use rusty_mill_board::board::{Board, Roles};
use rusty_mill_board::domain::{Actor, BoardError, Capability, Column, MoveRules};
use rusty_mill_board::roadmap::Lane;
use rusty_mill_board::store::MemStore;
use rusty_mill_board::workflow::Workflow;
use uuid::Uuid;

const FACTORY: &str = include_str!("../deploy/factory.workflow.json");

fn factory() -> Workflow {
    serde_json::from_str(FACTORY).expect("the factory workflow is valid")
}

fn col(id: &str) -> Column {
    Column::new(id).expect("id")
}

fn board() -> Board<MemStore> {
    Board::new(MemStore::default(), Roles::default()).with_workflow(factory())
}

fn who(b: &Board<MemStore>, name: &str, role: &str) -> Actor {
    b.roles().actor(name, role).expect("role")
}

/// A card a person approved into Ready.
fn approved(b: &Board<MemStore>, title: &str) -> Uuid {
    let h = who(b, "nano", "human");
    let c = b.add(&h, title, "", vec![]).expect("add");
    b.move_card(c.id, &col("ready"), &h).expect("approve");
    c.id
}

/// Put a card straight into `column` as the human, from Blocked (the triage route).
fn place(b: &Board<MemStore>, id: Uuid, column: &str) {
    let h = who(b, "nano", "human");
    b.move_card(id, &Column::BLOCKED, &h).expect("block");
    b.move_card(id, &col(column), &h).expect("place");
}

#[test]
fn columns_are_the_factory_stages_in_order() {
    let ids: Vec<String> = factory()
        .columns()
        .iter()
        .map(|c| c.id.to_string())
        .collect();
    assert_eq!(
        ids,
        [
            "backlog",
            "ready",
            "planning",
            "architecting",
            "building",
            "v_and_ving",
            "done",
            "blocked"
        ]
    );
}

#[test]
fn roadmap_lanes_follow_the_stages() {
    let b = board();
    let h = who(&b, "nano", "human");
    let later = b.add(&h, "later", "", vec![]).expect("add").id;
    let next = approved(&b, "next");
    let now: Vec<Uuid> = ["planning", "architecting", "building", "v_and_ving"]
        .iter()
        .map(|c| {
            let id = approved(&b, c);
            place(&b, id, c);
            id
        })
        .collect();
    let map = b.roadmap("default").expect("roadmap");
    let lane = |l: Lane| -> Vec<Uuid> {
        map.lanes
            .iter()
            .find(|v| v.lane == l)
            .map(|v| v.cards.iter().map(|c| c.id).collect())
            .unwrap_or_default()
    };
    assert_eq!(lane(Lane::Later), [later]);
    assert_eq!(lane(Lane::Next), [next]);
    let mut got = lane(Lane::Now);
    got.sort();
    let mut want = now;
    want.sort();
    assert_eq!(got, want);
}

#[test]
fn gates_need_admin_whatever_the_role_holds() {
    let b = board();
    // A model handed `triage` and `merge` by a roles edit still cannot pass a gate.
    let mut roles = Roles::default();
    roles.0.insert(
        "lead".into(),
        [Capability::Triage, Capability::Merge, Capability::Review]
            .into_iter()
            .collect(),
    );
    b.set_roles(roles);
    let lead = who(&b, "model", "lead");
    let h = who(&b, "nano", "human");
    let c = b.add(&h, "c", "", vec![]).expect("add");
    let gate = Err(BoardError::Forbidden(Capability::Admin));
    assert_eq!(b.move_card(c.id, &col("ready"), &lead).map(drop), gate);

    let design = approved(&b, "design");
    place(&b, design, "architecting");
    assert_eq!(b.move_card(design, &col("building"), &lead).map(drop), gate);
    b.set_move_rules(MoveRules::Open);
    assert_eq!(
        b.move_card(design, &col("building"), &lead).map(drop),
        gate,
        "gates hold with the movement rules off"
    );
    b.move_card(design, &col("building"), &h)
        .expect("the human approves the design");
}

#[test]
fn merging_is_a_gate_but_close_records_a_forge_merge() {
    let b = board();
    let h = who(&b, "nano", "human");
    let builder = who(&b, "claude", "builder");
    let reviewer = who(&b, "codex", "reviewer");
    let merged = approved(&b, "merged on the forge");
    place(&b, merged, "v_and_ving");
    b.claim(merged, &reviewer).expect("claim");
    assert_eq!(
        b.move_card(merged, &Column::DONE, &reviewer).map(drop),
        Err(BoardError::Forbidden(Capability::Merge)),
        "a reviewer never merges"
    );
    b.release(merged, &reviewer).expect("release");
    b.claim(merged, &builder).expect("claim");
    b.move_card(merged, &Column::DONE, &builder)
        .expect("close records a PR already merged");
    let gated = approved(&b, "merge here");
    place(&b, gated, "v_and_ving");
    b.move_card(gated, &Column::DONE, &h)
        .expect("the human merges");
}

#[test]
fn take_routes_each_role_to_its_stages() {
    let b = board();
    let planner = who(&b, "p", "planner");
    let architect = who(&b, "a", "architect");
    let tester = who(&b, "t", "tester");
    let builder = who(&b, "b", "builder");
    let reviewer = who(&b, "r", "reviewer");

    let id = approved(&b, "feature");
    assert!(b.take(&architect, None).expect("take").is_none());
    assert!(b.take(&builder, None).expect("take").is_none());
    let got = b.take(&planner, None).expect("take").expect("card");
    assert_eq!(
        (got.id, got.column, got.claim.as_deref()),
        (id, col("planning"), Some("p"))
    );
    b.release(id, &planner).expect("release");

    // Requirements written: the reviewer picks it up and passes it to design.
    let got = b.take(&reviewer, None).expect("take").expect("card");
    assert_eq!(got.id, id);
    b.move_card(id, &col("architecting"), &reviewer)
        .expect("reviewer pass");
    assert!(b.take(&planner, None).expect("take").is_none());
    assert_eq!(
        b.take(&architect, None).expect("take").map(|c| c.id),
        Some(id)
    );
    b.release(id, &architect).expect("release");

    let h = who(&b, "nano", "human");
    b.move_card(id, &col("building"), &h).expect("gate 1");
    assert!(b.take(&architect, None).expect("take").is_none());
    assert_eq!(b.take(&tester, None).expect("take").map(|c| c.id), Some(id));
    b.release(id, &tester).expect("release");
    assert_eq!(
        b.take(&builder, None).expect("take").map(|c| c.id),
        Some(id)
    );
    b.release(id, &builder).expect("release");

    b.claim(id, &reviewer).expect("claim");
    b.move_card(id, &col("v_and_ving"), &reviewer)
        .expect("reviewer pass on code");
    assert!(
        b.take(&builder, None).expect("take").is_none(),
        "V&V is the tester's"
    );
    assert_eq!(b.take(&tester, None).expect("take").map(|c| c.id), Some(id));
}

//! `FPL-FR-005` (ADR-0137): `split_card` interrupted between its steps
//! by a real `SIGKILL` (the `crash_safety` harness style, ADR-0095) —
//! the store must reopen valid with no dangling parent pointer, with
//! exactly the children whose insert returned before the kill, the
//! parent unchanged until its own replace returned, and the split
//! resumable from where it stopped.
#![cfg(unix)]

use rusty_fair_play_domain::{
    card_tree, chain_to_root, children_ordered, create_card_default_production_stack,
    create_card_production_stack, create_person_production_stack, deck_card_id,
    open_card_production_stack_portable, person_id, split_card, split_card_id, CardDefault, Origin,
    Person, SplitSpec, Suit, CARD_DEFAULT_FILE, CARD_FILE, PERSON_FILE,
};
use rusty_multimodal_db_engine::generic::query::{AllIds, GetById};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const CHILDREN: usize = 3;
const TRIALS: usize = 3;

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_dir(label: &str) -> TempDir {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "fair_play_crash_{label}_{}_{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    TempDir(dir)
}

fn writer() -> &'static str {
    env!("CARGO_BIN_EXE_fair_play_crash_writer")
}

/// Three deck cards, two people, no splits — written and dropped before
/// the writer runs, so it only ever opens.
fn seed(dir: &Path) {
    let defaults: Vec<CardDefault> = (1..=3)
        .map(|n| CardDefault {
            id: rusty_fair_play_domain::card_default_id(n),
            number: n,
            name: format!("Card {n}"),
            suit: Suit::Home,
            conception: "c".into(),
            planning: "p".into(),
            execution: "e".into(),
            minimum_standard_of_care: vec!["s".into()],
        })
        .collect();
    let cards = defaults.iter().map(CardDefault::to_card).collect();
    drop(create_card_default_production_stack(defaults, &dir.join(CARD_DEFAULT_FILE)).unwrap());
    drop(create_card_production_stack(cards, &dir.join(CARD_FILE)).unwrap());
    let people = ["Ada", "Bob"]
        .iter()
        .enumerate()
        .map(|(i, name)| Person {
            id: person_id(name),
            name: name.to_string(),
            player: i as u32 + 1,
        })
        .collect();
    drop(create_person_production_stack(people, &dir.join(PERSON_FILE)).unwrap());
}

/// Spawn the writer, read its stdout until `target_line`, `SIGKILL` it.
fn spawn_and_kill_on(dir: &Path, target_line: &str) {
    let mut child = Command::new(writer())
        .args([
            dir.as_os_str().to_str().unwrap(),
            &CHILDREN.to_string(),
            "sleep",
        ])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    for line in BufReader::new(stdout).lines() {
        if line.unwrap() == target_line {
            break;
        }
    }
    child.kill().unwrap();
    let status = child.wait().unwrap();
    assert!(
        !status.success(),
        "the writer finished before it could be killed: {status:?}"
    );
}

/// Every parent pointer resolves, every chain reaches deck card 1, and
/// the children present are a prefix of the planned ones in order.
fn assert_valid_and_count_children(dir: &Path) -> (usize, bool) {
    let cards = open_card_production_stack_portable(&dir.join(CARD_FILE)).unwrap();
    for id in cards.all_ids() {
        let card = cards.get(id).unwrap();
        if let Some(parent) = card.parent_card_id {
            assert!(
                cards.get(parent).is_some(),
                "card {id} points at a missing parent {parent}"
            );
            assert_eq!(
                chain_to_root(&cards, id).unwrap().last(),
                Some(&deck_card_id(1))
            );
            assert_eq!(card.origin, Origin::Family);
            assert_eq!(card.number, None);
        }
    }
    let kids = children_ordered(&cards, deck_card_id(1));
    for (i, kid) in kids.iter().enumerate() {
        assert_eq!(
            kid.id,
            split_card_id(&format!("1/split/{}", i + 1)),
            "children land in order"
        );
        assert_eq!(kid.position, i as u32);
        assert_eq!(
            kid.minimum_standard_of_care,
            vec![format!("standard {}", i + 1)]
        );
    }
    assert_eq!(
        card_tree(&cards, deck_card_id(1)).unwrap().children.len(),
        kids.len()
    );
    let parent = cards.get(deck_card_id(1)).unwrap();
    let parent_replaced = parent.owner_id == Some(person_id("Ada"));
    assert_eq!(parent_replaced, parent.notes == "split by the crash writer");
    (kids.len(), parent_replaced)
}

#[test]
fn an_uninterrupted_split_lands_whole() {
    let dir = temp_dir("control");
    seed(&dir.0);
    let status = Command::new(writer())
        .args([
            dir.0.as_os_str().to_str().unwrap(),
            &CHILDREN.to_string(),
            "exit",
        ])
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(assert_valid_and_count_children(&dir.0), (CHILDREN, true));
}

#[test]
fn a_split_killed_between_steps_reopens_valid_with_exactly_the_steps_that_returned() {
    let mut kill_points: Vec<(String, usize, bool)> = (1..=CHILDREN)
        .map(|k| (format!("CHILD_INSERTED {k}"), k, false))
        .collect();
    kill_points.push(("PARENT_REPLACED".into(), CHILDREN, true));
    for (marker, expect_children, expect_parent) in kill_points {
        for trial in 0..TRIALS {
            let dir = temp_dir("kill");
            seed(&dir.0);
            spawn_and_kill_on(&dir.0, &marker);
            let (children, parent) = assert_valid_and_count_children(&dir.0);
            assert_eq!(
                (children, parent),
                (expect_children, expect_parent),
                "killed after `{marker}` (trial {trial})"
            );
        }
    }
}

/// The documented resume: a rerun with the same ids is `Duplicate` on
/// the first child already present, so a caller skips those and splits
/// the rest; the store ends exactly where the uninterrupted run does.
#[test]
fn an_interrupted_split_is_resumed_by_skipping_the_children_already_present() {
    let dir = temp_dir("resume");
    seed(&dir.0);
    spawn_and_kill_on(&dir.0, "CHILD_INSERTED 2");
    assert_eq!(assert_valid_and_count_children(&dir.0), (2, false));
    let mut cards = open_card_production_stack_portable(&dir.0.join(CARD_FILE)).unwrap();
    let remaining: Vec<SplitSpec> = (1..=CHILDREN)
        .map(|i| SplitSpec {
            id: split_card_id(&format!("1/split/{i}")),
            name: format!("split {i}"),
            owner_id: (i % 2 == 1).then(|| person_id("Ada")),
            minimum_standard_of_care: vec![format!("standard {i}")],
            ..SplitSpec::default()
        })
        .filter(|spec| cards.get(spec.id).is_none())
        .collect();
    assert_eq!(remaining.len(), 1);
    let mut parent = cards.get(deck_card_id(1)).unwrap();
    parent.owner_id = Some(person_id("Ada"));
    parent.notes = "split by the crash writer".into();
    split_card(&mut cards, deck_card_id(1), remaining, Some(parent)).unwrap();
    drop(cards);
    assert_eq!(assert_valid_and_count_children(&dir.0), (CHILDREN, true));
}

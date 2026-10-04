//! `FPL-FR-006` (ADR-0137): the seed loader against the supplied deck —
//! shares its logic with `examples/fair_play_seed.rs` via `#[path]`, so
//! this exercises the code the CLI runs. Proves the 100-card load, the
//! suit and uniqueness checks, line-numbered refusals, idempotency by
//! number with a family's edits surviving a rerun, and splits by path.

#[path = "../examples/support/fair_play_seed_lib.rs"]
mod loader;

use loader::{parse_csv, seed, SeedError, SeedInputs, DECK_SIZE};
use rusty_multimodal_db::generic::fair_play::{
    balance, cards_by_suit, cards_held_by, children_ordered, deck_card_id,
    open_card_default_production_stack_portable, open_card_production_stack_portable,
    open_person_production_stack_portable, person_id, replace_card, split_card_id, state_counts,
    CardState, Suit, CARD_DEFAULT_FILE, CARD_FILE, PERSON_FILE,
};
use rusty_multimodal_db::generic::query::GetById;
use std::path::{Path, PathBuf};

fn unique_dir(label: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("fair_play_seed_{label}_{}_{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join(name)
}

fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn csv_parser_handles_quotes_doubled_quotes_and_crlf() {
    let rows = parse_csv("a,b\r\n1,\"x, \"\"y\"\"\"\n\"multi\nline\",\n").unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1].fields, vec!["1", "x, \"y\""]);
    assert_eq!(rows[2].fields, vec!["multi\nline", ""]);
    assert_eq!(rows[2].line, 3);
    assert_eq!(parse_csv("a,\"open\n").unwrap_err().0, 1);
}

#[test]
fn the_supplied_deck_loads_verbatim_and_a_rerun_creates_nothing() {
    let dir = unique_dir("deck");
    let store = dir.join("store");
    let inputs = SeedInputs {
        people: Some(write(&dir, "people.csv", "name\nAda\nBob\n")),
        cards: data("fair-play-cards.csv"),
        splits: None,
    };
    let report = seed(&store, &inputs).unwrap();
    assert_eq!(
        (
            report.people.created,
            report.card_defaults.created,
            report.cards.created
        ),
        (2, DECK_SIZE, DECK_SIZE)
    );
    assert_eq!(report.splits.created, 0);

    let cards = open_card_production_stack_portable(&store.join(CARD_FILE)).unwrap();
    let defaults =
        open_card_default_production_stack_portable(&store.join(CARD_DEFAULT_FILE)).unwrap();
    let people = open_person_production_stack_portable(&store.join(PERSON_FILE)).unwrap();
    assert_eq!(people.get(person_id("Bob")).unwrap().player, 2);
    let seventeen = cards.get(deck_card_id(17)).unwrap();
    assert_eq!(seventeen.name, "Meals (Weekday Dinner)");
    assert_eq!(seventeen.suit, Suit::Home);
    assert_eq!(seventeen.minimum_standard_of_care.len(), 3);
    assert_eq!(
        seventeen.minimum_standard_of_care[0],
        "Weekly dinner plan exists by Sunday"
    );
    assert_eq!(seventeen.notes, "");
    assert_eq!(
        cards.get(deck_card_id(53)).unwrap().notes,
        "Each partner holds their own copy of this card"
    );
    assert_eq!(cards_by_suit(&cards, Suit::UnicornSpace).len(), 2);
    assert_eq!(cards_by_suit(&cards, Suit::Wild).len(), 10);
    let counts = state_counts(&cards, &defaults).unwrap();
    assert_eq!(
        counts.total[&CardState::Original],
        DECK_SIZE,
        "freshly seeded: all Original"
    );
    drop((cards, defaults, people));

    // A family edit and a deal, then a rerun: nothing created, nothing overwritten.
    let mut cards = open_card_production_stack_portable(&store.join(CARD_FILE)).unwrap();
    let mut edited = cards.get(deck_card_id(17)).unwrap();
    edited.execution = "takeout is fine on Fridays".into();
    edited.owner_id = Some(person_id("Ada"));
    replace_card(&mut cards, edited).unwrap();
    drop(cards);
    let rerun = seed(&store, &inputs).unwrap();
    assert_eq!((rerun.cards.created, rerun.cards.existing), (0, DECK_SIZE));
    assert_eq!(
        (rerun.card_defaults.created, rerun.card_defaults.existing),
        (0, DECK_SIZE)
    );
    assert_eq!((rerun.people.created, rerun.people.existing), (0, 2));
    let cards = open_card_production_stack_portable(&store.join(CARD_FILE)).unwrap();
    let defaults =
        open_card_default_production_stack_portable(&store.join(CARD_DEFAULT_FILE)).unwrap();
    let seventeen = cards.get(deck_card_id(17)).unwrap();
    assert_eq!(seventeen.execution, "takeout is fine on Fridays");
    assert_eq!(seventeen.owner_id, Some(person_id("Ada")));
    assert_eq!(
        state_counts(&cards, &defaults).unwrap().total[&CardState::Edited],
        1
    );
}

#[test]
fn splits_resolve_by_path_and_owner_name_in_file_order() {
    let dir = unique_dir("splits");
    let store = dir.join("store");
    let splits = write(
        &dir,
        "splits.csv",
        "parent_path,name,owner_name,minimum_standard_of_care\n\
         2,Bathrooms,Ada,Scrubbed weekly|Towels swapped\n\
         2,Floors,Bob,\n\
         2/Floors,Mopping,,Mopped Sundays\n\
         2/Floors,Vacuuming,Ada,\n",
    );
    let inputs = SeedInputs {
        people: Some(write(&dir, "people.csv", "name\nAda\nBob\n")),
        cards: data("fair-play-cards.csv"),
        splits: Some(splits.clone()),
    };
    assert_eq!(seed(&store, &inputs).unwrap().splits.created, 4);
    let cards = open_card_production_stack_portable(&store.join(CARD_FILE)).unwrap();
    let kids = children_ordered(&cards, deck_card_id(2));
    assert_eq!(
        kids.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        vec!["Bathrooms", "Floors"]
    );
    assert_eq!(kids[0].id, split_card_id("2/Bathrooms"));
    assert_eq!(kids[0].owner_id, Some(person_id("Ada")));
    assert_eq!(kids[0].suit, Suit::Home, "copied from the parent");
    assert_eq!(
        kids[0].minimum_standard_of_care,
        vec!["Scrubbed weekly", "Towels swapped"]
    );
    let floors = children_ordered(&cards, split_card_id("2/Floors"));
    assert_eq!(
        floors
            .iter()
            .map(|c| (c.name.as_str(), c.position))
            .collect::<Vec<_>>(),
        vec![("Mopping", 0), ("Vacuuming", 1)]
    );
    assert_eq!(floors[0].owner_id, None, "empty owner_name is unassigned");
    assert_eq!(cards_held_by(&cards, person_id("Ada")).len(), 2);
    assert_eq!(
        balance(&cards, &[person_id("Ada"), person_id("Bob")], true),
        vec![(person_id("Ada"), 2), (person_id("Bob"), 0)]
    );
    drop(cards);
    // Rerun: all four exist, none duplicated.
    let rerun = seed(&store, &inputs).unwrap();
    assert_eq!((rerun.splits.created, rerun.splits.existing), (0, 4));
    let cards = open_card_production_stack_portable(&store.join(CARD_FILE)).unwrap();
    assert_eq!(children_ordered(&cards, deck_card_id(2)).len(), 2);
}

#[test]
fn malformed_rows_are_refused_by_line_and_nothing_is_written() {
    let dir = unique_dir("malformed");
    let store = dir.join("store");
    let deck = std::fs::read_to_string(data("fair-play-cards.csv")).unwrap();
    let cards_path = |label: &str, text: &str| write(&dir, &format!("{label}.csv"), text);
    let run = |cards: PathBuf, people: Option<PathBuf>, splits: Option<PathBuf>| {
        seed(
            &store,
            &SeedInputs {
                people,
                cards,
                splits,
            },
        )
    };
    let line_of = |err: SeedError| match err {
        SeedError::Row { line, .. } => line,
        other => panic!("expected a row error, got {other}"),
    };

    // A bad suit on line 5 (the header is line 1).
    let bad = deck.replacen(",Home,", ",Hom,", 1);
    assert_eq!(
        line_of(run(cards_path("suit", &bad), None, None).unwrap_err()),
        2
    );
    // A repeated number: the second occurrence's line.
    let dup = deck.replacen("\n2,Cleaning", "\n1,Cleaning", 1);
    assert_eq!(
        line_of(run(cards_path("dup", &dup), None, None).unwrap_err()),
        3
    );
    // 99 rows: a file-level refusal.
    let short: String = deck.lines().take(100).map(|l| format!("{l}\n")).collect();
    assert!(matches!(
        run(cards_path("short", &short), None, None),
        Err(SeedError::File { .. })
    ));
    // Wrong suit counts with the right row count: one Wild card made Home.
    let shifted = deck.replacen(",Wild,", ",Home,", 1);
    assert!(matches!(
        run(cards_path("shifted", &shifted), None, None),
        Err(SeedError::File { .. })
    ));
    // A people row with an empty name, line 3; and a duplicate.
    assert_eq!(
        line_of(
            run(
                data("fair-play-cards.csv"),
                Some(write(&dir, "p1.csv", "name\nAda\n \n")),
                None
            )
            .unwrap_err()
        ),
        3
    );
    assert_eq!(
        line_of(
            run(
                data("fair-play-cards.csv"),
                Some(write(&dir, "p2.csv", "name\nAda\nAda\n")),
                None
            )
            .unwrap_err()
        ),
        3
    );
    // A wrong header.
    assert_eq!(
        line_of(
            run(
                data("fair-play-cards.csv"),
                Some(write(&dir, "p3.csv", "person\nAda\n")),
                None
            )
            .unwrap_err()
        ),
        1
    );
    assert!(
        !store.join(CARD_FILE).exists(),
        "nothing written before the inputs pass"
    );

    // Splits are checked after the deck is loaded: an unknown owner, an
    // unknown child name, and a path not starting with a number.
    let header = "parent_path,name,owner_name,minimum_standard_of_care\n";
    let people = write(&dir, "p4.csv", "name\nAda\n");
    let s1 = write(&dir, "s1.csv", &format!("{header}2,Bathrooms,Zed,\n"));
    assert_eq!(
        line_of(run(data("fair-play-cards.csv"), Some(people.clone()), Some(s1)).unwrap_err()),
        2
    );
    let s2 = write(
        &dir,
        "s2.csv",
        &format!("{header}2,Bathrooms,Ada,\n2/Floors,Mopping,,\n"),
    );
    assert_eq!(
        line_of(run(data("fair-play-cards.csv"), Some(people.clone()), Some(s2)).unwrap_err()),
        3
    );
    let s3 = write(&dir, "s3.csv", &format!("{header}Cleaning,Bathrooms,,\n"));
    assert_eq!(
        line_of(run(data("fair-play-cards.csv"), Some(people), Some(s3)).unwrap_err()),
        2
    );
    // The deck and the one good split from s2 are there; the failed row is not.
    let cards = open_card_production_stack_portable(&store.join(CARD_FILE)).unwrap();
    assert_eq!(children_ordered(&cards, deck_card_id(2)).len(), 1);
}

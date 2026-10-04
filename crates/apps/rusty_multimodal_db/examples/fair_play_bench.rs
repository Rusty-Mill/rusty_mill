//! ADR-0137's two measurements, run as `cargo run --release --example
//! fair_play_bench`:
//!
//! 1. **Chain walk vs a denormalized root.** `root_card_id` walks
//!    `parent_card_id` one `GetById` per level; a stored `root_card_id`
//!    on every card would be one `GetById`. Measured at depths 1–5 on
//!    trees of a realistic size (the 100-card deck plus splits, up to a
//!    few hundred cards), the denormalized read is simulated as exactly
//!    one `GetById` of the leaf, which is what reading its own field
//!    would cost.
//! 2. **State queries as a scan.** `cards_by_state` and `state_counts`
//!    read every card and its baseline; measured at 100, 500, 2 000 and
//!    5 000 cards.
//!
//! Numbers are wall-clock medians over repeated runs on whatever machine
//! runs this; the ADR records one run and says so.

use rusty_multimodal_db::generic::fair_play::{
    card_tree, cards_by_state, create_card_default_production_stack, create_card_production_stack,
    deck_card_id, leaf_cards_under, root_card_id, split_card, state_counts, Card, CardDefault,
    CardState, SplitSpec, Suit,
};
use rusty_multimodal_db::generic::query::GetById;
use std::time::{Duration, Instant};
use uuid::Uuid;

fn deck(n: usize) -> Vec<CardDefault> {
    (1..=n)
        .map(|i| CardDefault {
            id: Uuid::from_u128(0x1000 + i as u128),
            number: u16::try_from(i).unwrap_or(u16::MAX),
            name: format!("Card {i} with a realistic name"),
            suit: Suit::ALL[i % 6],
            conception: "Notice what dinners need to work around: schedules, preferences, leftovers.".into(),
            planning: "Plan the week's dinners, match the grocery list, and decide cooking versus takeout nights.".into(),
            execution: "Cook, serve, clean up, and manage leftovers.".into(),
            minimum_standard_of_care: vec![
                "Weekly dinner plan exists by Sunday".into(),
                "Dinner on the table by an agreed time".into(),
                "Leftovers stored properly and used within 3 days".into(),
            ],
        })
        .collect()
}

fn spec(key: &str) -> SplitSpec {
    SplitSpec {
        id: rusty_multimodal_db::generic::fair_play::split_card_id(key),
        name: key.to_string(),
        conception: "conceive".into(),
        planning: "plan".into(),
        execution: "execute".into(),
        minimum_standard_of_care: vec!["done".into()],
        ..SplitSpec::default()
    }
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

fn time<R>(iters: usize, mut f: impl FnMut() -> R) -> Duration {
    let samples = (0..iters)
        .map(|_| {
            let t = Instant::now();
            std::hint::black_box(f());
            t.elapsed()
        })
        .collect();
    median(samples)
}

fn main() {
    let dir = std::env::temp_dir().join(format!("fair_play_bench_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // --- 1. chain walk vs denormalized root ---------------------------
    // 100 deck cards; under deck card 1 a chain of depth 5 with two
    // siblings at every level (a realistic "split, then split again"),
    // under cards 2–40 one split of three each: ~230 cards in all.
    let mut defaults = deck(100);
    for d in &mut defaults {
        d.id = Uuid::from_u128(0x2000 + u128::from(d.number));
    }
    let mut cards: Vec<Card> = defaults.iter().map(CardDefault::to_card).collect();
    for c in &mut cards {
        c.id = deck_card_id(c.number.unwrap());
    }
    let mut stack = create_card_production_stack(cards, &dir.join("chain.mmap")).unwrap();
    let mut parent = deck_card_id(1);
    let mut leaf_at_depth = Vec::new();
    for level in 1..=5 {
        let a = spec(&format!("chain/{level}/a"));
        let b = spec(&format!("chain/{level}/b"));
        let next = a.id;
        split_card(&mut stack, parent, vec![a, b], None).unwrap();
        leaf_at_depth.push(next);
        parent = next;
    }
    for n in 2..=40u16 {
        let specs = (0..3).map(|i| spec(&format!("{n}/{i}"))).collect();
        split_card(&mut stack, deck_card_id(n), specs, None).unwrap();
    }
    let total = rusty_multimodal_db::generic::query::AllIds::<Card>::all_ids(&stack).len();
    println!("## chain walk vs denormalized root ({total} cards)\n");
    println!("| depth | chain walk (`root_card_id`) | one `GetById` (denormalized) | ratio |");
    println!("|---|---|---|---|");
    for (i, leaf) in leaf_at_depth.iter().enumerate() {
        let walk = time(2_000, || root_card_id(&stack, *leaf).unwrap());
        let one = time(2_000, || stack.get(*leaf).unwrap().parent_card_id);
        println!(
            "| {} | {:?} | {:?} | {:.1}× |",
            i + 1,
            walk,
            one,
            walk.as_secs_f64() / one.as_secs_f64()
        );
    }
    let tree = time(500, || card_tree(&stack, deck_card_id(1)).unwrap());
    let leaves = time(500, || leaf_cards_under(&stack, deck_card_id(1)).unwrap());
    println!(
        "\n`card_tree` of the depth-5 root (11 cards): {tree:?}; `leaf_cards_under`: {leaves:?}\n"
    );

    // --- 2. state queries as a scan -----------------------------------
    println!("## state queries, a scan\n");
    println!("| cards | `cards_by_state(Edited)` | `state_counts` |");
    println!("|---|---|---|");
    for &n in &[100usize, 500, 2_000, 5_000] {
        let defaults = deck(n);
        let mut cards: Vec<Card> = defaults.iter().map(CardDefault::to_card).collect();
        for (i, c) in cards.iter_mut().enumerate() {
            c.id = Uuid::from_u128(0x3000 + i as u128);
            if i % 10 == 0 {
                c.planning.push('!'); // one in ten is Edited
            }
        }
        let defaults_stack =
            create_card_default_production_stack(defaults, &dir.join(format!("d{n}.mmap")))
                .unwrap();
        let cards_stack =
            create_card_production_stack(cards, &dir.join(format!("c{n}.mmap"))).unwrap();
        let iters = if n >= 2_000 { 20 } else { 200 };
        let by_state = time(iters, || {
            cards_by_state(&cards_stack, &defaults_stack, CardState::Edited).unwrap()
        });
        let counts = time(iters, || {
            state_counts(&cards_stack, &defaults_stack).unwrap()
        });
        println!("| {n} | {by_state:?} | {counts:?} |");
    }
    std::fs::remove_dir_all(&dir).ok();
}

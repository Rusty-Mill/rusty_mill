//! The child process `tests/fair_play_crash.rs` spawns and `SIGKILL`s
//! mid-`split_card` (`FPL-FR-005`, ADR-0137) — the `crash_writer`
//! pattern (ADR-0095): a real subprocess kill, so no `Drop` or unwind
//! runs, with a flushed marker line and a deliberate pause after each
//! durable step so the harness lands the kill in a chosen gap.
//!
//! Usage: `fair_play_crash_writer <store_dir> <children> <exit|sleep>`
//!
//! Opens the three stacks the harness created under `<store_dir>`,
//! splits deck card 1 into `<children>` cards (`split/<i>`, the odd ones
//! handed to `Ada`) and hands the parent itself to `Ada` with a note,
//! printing `CHILD_INSERTED <i>` after each child and `PARENT_REPLACED`
//! after the parent, each followed by a 150 ms pause; then `ALL_DONE`,
//! and either exits `0` (the control run) or sleeps forever (so the kill
//! always lands on a live process).

use rusty_fair_play_domain::{
    deck_card_id, open_card_production_stack_portable, person_id, split_card_id, split_card_with,
    SplitSpec, SplitStep, CARD_FILE,
};
use rusty_multimodal_db_engine::generic::query::GetById;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

fn print_line(line: &str) {
    println!("{line}");
    std::io::stdout().flush().expect("flush stdout");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, dir, children, mode] = args.as_slice() else {
        eprintln!("usage: fair_play_crash_writer <store_dir> <children> <exit|sleep>");
        std::process::exit(2);
    };
    let children: usize = children.parse().expect("children must be a number");
    let mut cards =
        open_card_production_stack_portable(&Path::new(dir).join(CARD_FILE)).expect("open cards");
    let parent_id = deck_card_id(1);
    let mut parent = cards.get(parent_id).expect("deck card 1 is seeded");
    parent.owner_id = Some(person_id("Ada"));
    parent.notes = "split by the crash writer".into();
    let specs = (1..=children)
        .map(|i| SplitSpec {
            id: split_card_id(&format!("1/split/{i}")),
            name: format!("split {i}"),
            owner_id: (i % 2 == 1).then(|| person_id("Ada")),
            minimum_standard_of_care: vec![format!("standard {i}")],
            ..SplitSpec::default()
        })
        .collect();
    let mut inserted = 0;
    split_card_with(&mut cards, parent_id, specs, Some(parent), |step| {
        match step {
            SplitStep::ChildInserted(_) => {
                inserted += 1;
                print_line(&format!("CHILD_INSERTED {inserted}"));
            }
            SplitStep::ParentReplaced => print_line("PARENT_REPLACED"),
        }
        std::thread::sleep(Duration::from_millis(150));
    })
    .expect("split");
    print_line("ALL_DONE");
    if mode == "exit" {
        return;
    }
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

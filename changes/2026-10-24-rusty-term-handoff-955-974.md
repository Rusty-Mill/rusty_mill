---
category: Changed
changelog: The rusty_term handoff docs record that rusty_term no longer depends on tokio (#955) and that the view-row map (backlog row 9, #952) and short frame lock (row 8, #974) landed.
---
## 2026-10-24 - rusty_term handoff docs: tokio removed, rows 9 and 8 done

- **Changed:** `docs/projects/mux/RUSTY-TERM-NOTES.md` and `RUSTY-TERM-HANDOFF.md` no longer say the `rusty_term` front-end crate depends on tokio. #955 moved it to `rusty_tokio`, and `cargo tree -p rusty_term --all-features -i tokio` finds none. The mux still links `rusty_term_core`. `parking_lot` remains until #956 lands.
- **Changed:** roadmap steps 1 and 2 are marked done: backlog row 9 (view-row map once per frame) in #952, and row 8 (short lock) in #974, each with its own local numbers. The pre-fix fold-block measurements are labelled historical. Row 7 (dirty rows) remains, and so does fair handoff for the lock.
- **Known limitations:** documentation only. The lock-hold figures are #974's local measurements.

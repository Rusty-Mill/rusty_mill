---
category: Changed
changelog: The rusty_term handoff docs record that rusty_term no longer depends on tokio (#955) and that the short frame lock (backlog row 8) landed (#974).
---
## 2026-10-24 - rusty_term handoff docs: tokio removed, short lock done

- **Changed:** `docs/projects/mux/RUSTY-TERM-NOTES.md` and `RUSTY-TERM-HANDOFF.md` no longer say the `rusty_term` front-end crate depends on tokio. #955 moved it to `rusty_tokio`, and `cargo tree -p rusty_term --all-features -i tokio` finds none. The mux still links `rusty_term_core`. `parking_lot` remains until #956 lands.
- **Changed:** roadmap step 2 (backlog row 8, short lock) is marked done in #974, with #974's local lock-hold numbers and what it left open (no fair handoff). Rows 9 and 7 remain.
- **Known limitations:** documentation only. The lock-hold figures are #974's local measurements.

---
category: Changed
changelog: The async runtime inventory marks gaps G2-G5 and G8 closed; they shipped in #997.
---
## 2026-10-25 - Async runtime inventory: G2-G5 and G8 closed

- **Changed:** `docs/research/ASYNC-RUNTIME-INVENTORY.md` marks gaps G2 and G3 (#898), G4 and G5 (#899) and G8 (#901) closed. The code shipped in #997; #899 and #901 had stayed open.
- **Verified:** on `main`, each acceptance criterion of #899 and #901 has a passing test. `tests/main_test_attrs.rs`, `tests/cancellation_token.rs` and `tests/pause.rs` pass, and so do the `MacroArgsCompileFail` doctests, which include the unknown-flavor case.

---
category: Added
changelog: rusty_erosion measures Python through measure_in for the #705 SlopCodeBench spike; the gate stays Rust-only.
---
## 2026-11-26 - rusty_erosion: Python adapter for the #705 spike

- **Added:** `adapter/python.rs` (tree-sitter-python 0.25, approved as D202) with radon-style per-function CC and normalized tokens, plus `measure_in` and `classify_in` to choose the measured languages. Python test files (`tests/`, `test_*.py`, `*_test.py`, `conftest.py`) are ignored only when Python is measured; the Rust-only report still lists them as unmeasured.
- **Changed:** `FileClass::Rust` is now `FileClass::Measured(Lang)`. `measure`, `classify` and the `erosion check` gate still measure Rust only, so gate results and calibration do not change.
- **Known limitations:** f-string interpolations count as one `STR` token; module-level code adds tokens but no CC.

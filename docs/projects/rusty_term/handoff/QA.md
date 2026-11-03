# Implementer Q&A — rusty_term (with rusty_mux)
Harvested packs start empty. `rusty-term-expert` adds a question here only when it was actually asked and answered from sources, with its tag and source.

## Where do regression tests for the uncapped WriteConsoleInput queue (D56) go?
Asked 2026-11-03 (smoke test). **In-module `#[cfg(test)]` tests**, next to the code: `Queue` tests in `src/input.rs` `mod tests` (beside `the_queue_drops_the_oldest_past_the_cap_and_streams_repeats`), console-level WriteConsoleInput tests in `src/console/read.rs` `mod tests` (beside `terminal_input_past_the_cap_displaces_the_oldest_written_records`). `tests/` holds only Windows end-to-end `OwnPty` runs; `Queue`, `MAX_RECORDS` and `mod input` are crate-private, so an integration test would need a public API change (a Nano gate). Tag: answered.
Source: `docs/projects/term/conhost/design.md` §3.3 "Tests and adapters"; `crates/libs/ui/rusty_conhost/src/input.rs:23,778,913`; `src/lib.rs:30`; `tests/own_pty.rs:1-3`.

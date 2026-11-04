# Implementer Q&A — rusty_term (with rusty_mux)
Harvested packs start empty. `rusty-term-expert` adds a question here only when it was actually asked and answered from sources, with its tag and source.

## Where do regression tests for the uncapped WriteConsoleInput queue (D56) go?
Asked 2026-11-03 (smoke test). **In-module `#[cfg(test)]` tests**, next to the code: `Queue` tests in `src/input.rs` `mod tests` (beside `the_queue_drops_the_oldest_past_the_cap_and_streams_repeats`), console-level WriteConsoleInput tests in `src/console/read.rs` `mod tests` (beside `terminal_input_past_the_cap_displaces_the_oldest_written_records`). `tests/` holds only Windows end-to-end `OwnPty` runs; `Queue`, `MAX_RECORDS` and `mod input` are crate-private, so an integration test would need a public API change (a Nano gate). Tag: answered.
Source: `docs/projects/term/conhost/design.md` §3.3 "Tests and adapters"; `crates/libs/ui/rusty_conhost/src/input.rs:23,778,913`; `src/lib.rs:30`; `tests/own_pty.rs:1-3`.

## What constrains an O11 first-run screen's layout, telemetry and copy?
Asked 2026-11-04. **Layout and look** are GUI design: no `rusty-term-design-expert` file exists, so they escalate to Nano (D10). **Telemetry**: none; mux C09 "no telemetry" and CHARTER "no hidden egress" (the importers and agent detection read local files only). **Copy and behaviour**: STYLEGUIDE UX rules (copy never overclaims, one-line descriptions of at most 80 characters, keyboard first with Enter/Esc, quiet back-out, platform-matched labels, missing data offers a way to get it). Tag: answered.
Source: D10; `docs/projects/mux/CAPABILITIES.md` C09; `docs/projects/mux/CHARTER.md:7,21`; `docs/projects/term/STYLEGUIDE.md` "UX rules".

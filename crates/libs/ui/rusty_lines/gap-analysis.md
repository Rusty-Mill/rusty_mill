# gap-analysis.md — rusty_lines parity audit (2026-07-23)

## Scope decision (skill step 0)

`docs/survey.md` (mirrored verbatim in README.md's "Feature matrix" and
"Deliberate narrowings" sections) is a hand-curated, already-existing
scope document: 45 capability rows describing what's implemented, each
naming a reference editor whose semantics are matched, plus 8 "Deliberate
narrowings" describing what's consciously out of scope with a stated
reason. Per the parity-loop skill's step 0, this **is** the definition of
parity for rusty_lines — this run audits implementation status against it
rather than diffing against an external crate (there's no comparable
`cargo public-api` surface to diff against a "reference readline crate";
the reference is a field of *behaviors* across many editors, not a single
crate).

Source for every row below: `roadmap` (docs/survey.md).

## Method

Four parallel agents independently verified every capability-audit row
and every narrowing bullet against `src/lib.rs`, `tests/pty.rs`, and the
`tests` module, citing file:line evidence for what's actually there
(not just documented) — since the doc reads in present tense as if
already fully implemented, the risk this audit exists to catch is a
stale "done" claim, not a `cargo public-api` diff.

## Result: zero capability gaps

All 45 capability-audit rows are genuinely implemented, matching the
behavior described in `docs/survey.md`. No row was MISSING. No row
required a change to an existing public signature. There is nothing to
file as a `parity-gap` issue from the capability table.

All 8 "Deliberate narrowings" bullets were confirmed accurate (still
correctly absent, stated reason still holds), with one exception, fixed
directly rather than filed as an issue since it's a one-line doc
correction, not a code gap:

| Item | Finding | Resolution |
| --- | --- | --- |
| "Non-tty / non-Unix" narrowing | `docs/survey.md` still claimed "Non-Unix builds get a buffered prompt-and-read," which predates the Windows raw-mode backend added on top of `rusty_win32` (commit `d82af37`). README.md's copy of this section was already corrected (heading trimmed to "Non-tty", the stale sentence removed) but `docs/survey.md` was never updated to match, leaving the two mirrored docs contradicting each other and contradicting current `term_sys.rs`. | Fixed in this run — `docs/survey.md`'s narrowing bullet now matches README.md's wording. |

No half-finished or dead code was found for any narrowed-out feature
(no macros/redo/mark-region/`.inputrc`-parser scaffolding).

## Non-gap finding: test-coverage debt on already-correct capabilities

Not a parity gap (nothing here is missing or wrong per the survey's
description), but worth surfacing since the audit was already reading
every call site: several rows have implementations that fully match
`docs/survey.md`, confirmed correct by code inspection, but a specific
sub-behavior named in that row's own description has **no test**
(unit or pty) exercising it. Listed here rather than in the gap table
above because the parity-loop skill's issue mechanism (breaking-change
flag, "present in reference, missing here") doesn't fit "implemented
correctly, just unverified":

| Capability (survey row) | Untested sub-behavior | Evidence of correct impl |
| --- | --- | --- |
| Quoted insert (row 12) | `quoted_insert` (C-v/C-q inserting a literal control byte) has no test at all | `src/lib.rs:3907-3921` |
| Edit in `$VISUAL`/`$EDITOR` (row 13) | C-x C-e / vi `v` round-trip has no test at all | `src/lib.rs:3924-3990` |
| Pre-seeded lines (row 5) | `read_line_with_initial_timeout` (initial text *and* deadline together) untested as a combination | `src/lib.rs:1139-1148` |
| History persistence (row 18) | `#V2` rustyline-header skip on load never exercised by a file that actually has one | `src/lib.rs:986-988` |
| Bell on failed operations (row 22) | No test asserts the bell byte/flash is actually emitted at any of the 4+ call sites | `src/lib.rs:2600-2617` |
| Tab completion (row 26) | `completion_query_items` y/n threshold prompt and column-major layout output untested | `src/lib.rs:4193-4230` |
| Abbreviation expansion (row 30) | Plain non-edge-case "abbr" + space → expansion has no dedicated test (only an edge-case clamp test doubles for it) | `src/lib.rs:5596-5616` |
| Bracketed paste (row 32) | Multi-line paste's `⏎` display / "returns as a unit" / joined history entry has no single end-to-end pty test tying the pieces together | `src/lib.rs:1716-1780`, `906-909` |
| Robust escape decoding (row 37) | No test feeds an actual unrecognized/private-mode CSI sequence (e.g. an SGR mouse report) through the live decoder | `src/lib.rs:1743-1780` |
| Resize / self-heal (row 38) | SIGTSTP/`fg`/external-`stty` cooked-terminal re-assert branch has no test | `src/lib.rs:2228-2244` |
| Readline variables (row 41) | `set_completion_ignore_case`/`set_show_all_if_ambiguous` never set `true` in any test; `BellStyle::Audible`/`Visible` never exercised | `src/lib.rs:4156`, `4176`, `2600-2608` |
| Terminal facilities (row 45) | `terminal_size()` and `with_echo_disabled` have zero test coverage | `src/lib.rs:512-514`, `555-572` |

## Outcome

Per the skill's stop conditions: no open (or filable) `parity-gap`
issues exist after this assessment — rusty_lines is at full parity with
its own hand-curated scope document. The loop ends here for this run;
no issues were filed, no PRs opened for capability work.

The test-coverage list above was, per the user's direction, implemented
directly in this same session rather than filed as separate issues:
all 12 items now have unit or pty tests (see CHANGELOG.md's Unreleased
entry for the full list). No production code changed — every one of the
12 capabilities was already correct; only tests were added. Full local
gate (`cargo build && cargo test && cargo clippy --all-targets -- -D
warnings && cargo fmt --check`) is green: 83 unit tests, 43 pty tests, 1
doctest.

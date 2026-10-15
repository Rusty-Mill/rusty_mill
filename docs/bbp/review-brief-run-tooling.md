# Fresh-session review brief: BBP 2b run tooling

To be run in a session that did not write this code, before the pilot.

You are reviewing code you did not write, in a session that has not seen the author's reasoning. Keep it that way.

## Ground rules

- **Do not read** the author's PR descriptions, `changes/*.md` fragments, commit messages or chat. They state conclusions; you are here to test them. Read the code and the criteria note (`docs/bbp-concurrency-design.md`, section "2b gate criteria") only.
- Review the repository at the commit named in the pilot checklist, or `origin/main` plus the PR that adds the run guards. Do not edit the code under review. Report; do not fix.
- A finding needs a file:line, a reproduction (a test, a command, or a mutation that survives), and what you expected instead. "No finding" is an acceptable answer for an item, but say what you tried.
- Do not assume the tests are right. Several use hand-made numbers and hand-made log lines, not captured output.

## Scope

- `crates/apps/rusty_bbp_host/src/pairs.rs`: `evaluate`, `decide`, the per-criterion functions.
- `crates/apps/rusty_bbp_host/src/grade.rs`: the blind packs (`pack`, `sources`, `join`).
- `crates/apps/rusty_bbp_host/src/tokens.rs`: pricing (`Criteria::dollars`, `Usage::cache_writes`, `by_model`, `parse`, `reconcile`) and `docs/bbp/2b-criteria.json`.
- `crates/apps/rusty_bbp_host/src/pins.rs` and `moderator.rs`: the pre-run agents-file check and the live cost cap.

## Task 1: the decision rows

The decision table in the criteria note is applied by `decide`. For **each** row (noise gate, 1, 2, 3 inconclusive, 3 fails, 7 fails, 4 fails, 5 fails, all hold, incomplete):

1. Write a mutation that should change the outcome of that row alone: swap it with its neighbour, delete its condition, weaken it (`==` to `!=`, `Fails` to `Inconclusive`), or move it after a later row.
2. Run `cargo test -p rusty_bbp_host --lib pairs`. A mutation that no test catches is a finding.
3. Independently of the tests, compare `decide` with the table text in the note, row by row and in order. Any divergence in wording, order or precedence is a finding. Check specifically: what happens when criterion 3 is inconclusive and criterion 7 fails; when 1 and 2 both fail; when everything holds except criterion 4 is inconclusive.
4. Check that `evaluate` can actually produce each verdict the table needs: find inputs for which each per-criterion function returns `Holds`, `Fails` and `Inconclusive`, or say which verdicts are unreachable.

## Task 2: the blind grader

The grader must not be able to tell the arm, the pair, the run, the role or the log from what it is given.

1. Read `PackEntry` and what `pack` writes. List every field, and for each say what it could reveal.
2. Build a small synthetic manifest (two runs, one control and one treatment, with specs of your own) and run `bbp grade pack`. Diff `pack.json` against `key.json`. Look for anything in `pack.json` that correlates with the key: ids, order, lengths, rubric text, whitespace, spec text copied from artifact bytes (does `spec_markdown` strip all of the artifact framing?).
3. Try to defeat the blindness: run the pack twice with different input order and the same seed; with different seeds; with a run that has no unused versions against one that has three. Does the **count** of entries per task, or their order, leak the arm?
4. Check `join`: can the key be opened with partial scores? With extra ids? With the wrong number of verdicts? Can `lost_rubric_items` be computed for a control run?

## Task 3: pricing and the unpriced-model error

A run must never look cheap because a model was missing from the price table.

1. Enumerate every way a model name can arrive: the exact id, an id with a date suffix, a context suffix such as `[1m]`, an empty string, a missing field, different letter case, `unknown`. For each, trace it through `by_model` and `Criteria::dollars`. Each must either price correctly or return an error naming the model. Anything that prices at zero, or is dropped, is a finding.
2. Check double counting and under counting: the `result` line's usage against the sum of assistant messages, messages printed twice, messages with no id, messages with usage but no model, a killed harness with no `result`. Which figure is used for tokens and which for dollars, and can they disagree?
3. Check `reconcile`: a part that is zero on one side, exactly at 2%, just over; dollars present on one side only; `without_output` greater than zero.
4. Check the claim that the stream-json line shapes are right. The tests use shapes the author wrote from memory. Compare them with the Claude Code documentation for `--output-format stream-json` (fetch it) and say where they differ.
5. Read `docs/bbp/2b-criteria.json`: are the prices plausible against the current Anthropic price page, and is every model the harness might print in it?

## Task 4: the agents-file check and the cost cap

1. `pinned_model` finds `--model` in a launcher's program and arguments. Find a launcher that really pins a model and is **not** detected (a pin inside an environment variable, a wrapper script, a different flag spelling), and a launcher that is detected as pinned although the harness would not take that model (a `--model` inside a prompt string).
2. Can `bbp mod` start an evidence run with an unpinned role by any route (no `--criteria`, a role added after the check, a launcher edited between the check and the run)? Say which.
3. The cost cap re-reads harness output each poll. Find an input where the cap fails to stop a run that has spent more than the cap: output the parser skips, a model name that prices, a message printed after the check, a harness that writes elsewhere.
4. Cache writes: confirm that a missing or short `cache_creation` split can only raise the price relative to the printed split, and that the 5-minute and 1-hour prices in `docs/bbp/2b-criteria.json` are the ones the price page lists.

## What to hand back

A list of findings, most serious first, each with file:line, reproduction, and expected behaviour; then, per task, the mutations you ran and which survived. Say plainly if you ran out of time on an item.

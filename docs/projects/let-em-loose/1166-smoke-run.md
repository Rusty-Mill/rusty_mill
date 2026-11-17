# #1166 Smoke run: lens run on a toy claims task, local model first (step 1 design)

Status: design for Nano's approval (card #1166 step 1). Nothing is built, deployed or run. No model was called and the board was not read (the card text was not available to the author; pass criteria below follow the brief for this design and are marked where the card may differ).
Authority: D130 (M1 design batch), D131 (deploy on BaileyAi), **D134 (no paid runs for now: this run is local-only)**, D127 (freeze: nothing new), D53, D86, D93, D37.
Depends on: #1092 deployed (`1092-deploy-layout.md`) and #1123's `lens.json` and prompts in place (`1123-lens-prompts.md`).

## 1. What this run proves (and what it cannot)
Proves the claims path end to end on one host with local models only: `bbp open` (claims mode, frozen holdout) -> `rusty_orch lens run` (three proposals, blind reviews, settlement) -> an irreversible claim reaches the accept line and escalates -> the Mill Board inbox shows it -> a human tap writes the log -> `bbp why --claim`, `bbp verify` and `rusty_orch lens record` read it back.
Cannot prove (no builder in M1, D129): that an accept waits for a holdout pass (see 6), executed evidence (see 8), breaker behaviour (see 7), or anything about cloud lenses (D134).

## 2. The intent (frozen as the task brief, `brief.md`)
> **sweep v0.1-smoke.** A command `sweep DIR DAYS [--yes]`.
> - It looks only at regular files directly inside DIR whose modification time is more than DAYS days old.
> - Without `--yes` it prints `would delete <path>` for each and deletes nothing. With `--yes` it prints `delete <path>` and then deletes the file.
> - **Never:** follow a symlink, touch a subdirectory, act when DAYS is not a positive whole number (print usage, exit 2), or delete without `--yes`.
> - **Irreversible:** a deleted file is gone. There is no trash and no undo. Any claim whose action is running `sweep --yes` is irreversible and goes to Nano.
> - Nothing is built in this run. The run settles claims about how to build it.

Seed claims, posted once by the operator as principal `seed` (see 8 step 6):
- Question Q, domain `question`, stakes low, reversible: "How should sweep decide a file is old enough, and how can it show it deleted only those files?"
- Action claim A, domain `sweep`, stakes medium, **reversible: false**: "Run `sweep --yes` on the toy data directory with DAYS 30." Budget 16 x 2 = 32 units; accept line 12 (CL-54, C-R6).

## 3. Holdout file (frozen at open; D37, #597)
`holdout/sweep/test.sh`, plus `holdout-profiles.json` naming one profile `holdout:sweep` = `sh "$BBP_HOLDOUT/sweep/test.sh"` (the shape `holdout.rs` requires: a `holdout:` or `golden:` prefix, absolute non-empty root, outside any path a build run reads).

```sh
#!/bin/sh
# Written from the intent alone. Runs in the candidate checkout; ./sweep is the candidate.
set -u
fail=0; ok() { [ "$1" = 0 ] || { echo "FAIL: $2"; fail=1; }; }
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
mkdir "$T/d" "$T/d/sub" "$T/out"
touch -d '40 days ago' "$T/d/old" "$T/d/sub/old-in-sub" "$T/out/target-old"
touch "$T/d/new"; ln -s "$T/out/target-old" "$T/d/link"; touch -h -d '40 days ago' "$T/d/link"
./sweep "$T/d" 30 >"$T/dry"; ok $? "dry run exits 0"
[ -e "$T/d/old" ]; ok $? "dry run deleted nothing"
grep -q "would delete $T/d/old" "$T/dry"; ok $? "dry run names the old file"
./sweep "$T/d" 30 --yes >/dev/null; ok $? "--yes exits 0"
[ ! -e "$T/d/old" ]; ok $? "old file deleted"
[ -e "$T/d/new" ]; ok $? "new file kept"
[ -e "$T/d/sub/old-in-sub" ]; ok $? "subdirectory untouched"
[ -e "$T/out/target-old" ]; ok $? "symlink target untouched"
for bad in 0 -3 abc 2.5; do ./sweep "$T/d" "$bad" --yes >/dev/null 2>&1; [ $? = 2 ]; ok $? "DAYS $bad exits 2"; done
exit $fail
```

Toy repo (`/srv/lel/toy/repo`, throwaway git repo, outside any read root): a stub `sweep` that prints usage and exits 2, and `tests/usage.sh` that checks that exit. Profile set: one `test` profile `sh tests/usage.sh`, `test_paths: ["tests/**"]`, `"erosion": "off"` [default: the opt-out the open requires for non-Rust code; erosion measures Rust only, D45], read roots `/bin /usr /lib /lib64`, limits as `docs/proving/profiles.json`. No `--base`, so no baseline run.

## 4. Lenses and the local-only `lens.json`
**How:** a separate file, `lens-local.json`, next to `lens.json` in Factory and deployed to `/srv/lel/lens-local.json`. `lens check` and `lens run` take the config as their first argument, so no code change and no flag. `lens.json` (the mixed config of #1123) is untouched and is not used in this run. A third provider id is needed because the config refuses a rebel equal to either of the first two proposers (`check_proposers`).

| Lens | Does | Models (routine; contested) |
|---|---|---|
| architect | proposers `local`, `local-high`; rebel `local-rebel`; posts claims | local, local-high, local-rebel; local-high |
| adversary | counter and support evidence; group `attack` | local-high, local; local-high |
| operator | counter and support evidence; group `attack` | local, local-high; local-high, local |

`local` = Qwen3.6-35B-A3B, `local-high` = gpt-oss-120b, `local-rebel` = Qwen3.6-35B-A3B again under its own id (same weights; the rebel differs only by orch's rebel suffix, so the three candidates are not three independent models: noted in the record, not hidden). Adversary leads local-high and operator leads local, so `check_diversity` passes. All three are `openai-http` with `max_class` sensitive by default, price 0, `context_tokens` 32768, `base_url http://127.0.0.1:<port>/v1` (port and model ids unverified until step 1 of 8). **Research is omitted [default]:** its `tools: read-only` needs a CLI provider (`config.rs`) and without tools it has nothing it may cite, so it would post nothing.
`domain_classes` empty; ceilings `experiment_usd 1`, `task_usd 1`, `task_calls 60`; no `breaker` key. Prompts: the same `lenses/{architect,adversary,operator}.md`.

## 5. Budget and call estimate (all $0, local)
Calls: 3 proposals; each of 4 claims (3 candidates + A) gets adversary and operator: 1 blind each, plus look-harder if no counter, plus informed per wave: about 2 to 6 per claim. Expect 20 to 35, worst case ~50. Ceiling `task_calls 60` stops it (the meter and ceilings enforce in v0; money cannot trip at price 0). `--max-passes 8`. Claim budgets (addendum): Q 8 units, candidates 8 each, A 32. Accept 8 (low/medium reversible), 12 (A, irreversible), drop -8, `min_authors` 2, defaults from `ClaimsConfig` with no `--claims` file (#593).
Wall time unknown: llama-swap swaps one large model at a time (unverified), so calls serialise and swapping may dominate. Time box 90 minutes; stop and record if exceeded. No measured tokens exist yet; this run produces the first (feeds 1124 section 1).

## 6. Holdouts (#597): what the run can and cannot show
The accept line waits for a holdout pass only on a claim that has a **candidate** (`holdout_passed` in `claims_engine.rs` returns true with no candidate). Nothing in M1 stores a candidate: lenses post claims and evidence only, `#600` builders are after the first run (D129), and orch has no holdout lens (`holdout_due` is hard-coded false in its cards). So in this run the holdout is staged, hashed and frozen (the open proves the set is valid, isolated and non-empty) but never run, and no accept is held by it. The criterion "an accept waits for a holdout pass" is **deferred to the first #600 builder run**. See ESCALATE 1.

## 7. Does #758 (breaker wiring) have to come first? No.
- The breaker exists only as a pure module: `orch-lens/src/breaker.rs`, its config parse in `config.rs`, a `pub mod` in `lib.rs`. Nothing in `rusty_orch/src/lens/` calls it, and there is no `rusty_orch lens breaker` command (grep, 2026-11-16).
- `Mode` defaults to `Observe`; observe only reports a would-trip, and even that report is not emitted yet. QA Q4 / card #598 Q4: observe-only in the first run. `bbp pause --reason breaker` exists on the host and nothing calls it.
- `lens run` stops on: no job started, a halted round, `--max-passes`, and the `task_calls` ceiling. Those need nothing from #758.
- **Needs #758:** any pass criterion of the form "a would-trip is logged or reported", "breaker settings in config match the run", or "a trip pauses the task". Here: none of the criteria in 9 do. If the card's own list has one, mark it deferred.
- The card lists #758 under Waits on. Recommend removing that link for this run (a board edit; not made here). #758 items 1-3 stay in M1 (D128), just not as a precondition.

## 8. Order of operations on BaileyAi (after #1092 deploy and #1123 config)
All as `orch-runner`; no paid calls; every step stops the run on failure and records why.
1. **Config in place.** `/srv/lel/lens-local.json`, `/srv/lel/lenses/*.md`, `/srv/lel/toy/{repo,holdout,brief.md,roster.json,profiles.json,holdout-profiles.json}`. Confirm the llama-swap port and the two model ids (`curl http://127.0.0.1:<port>/v1/models`) and fix `lens-local.json`. Both are unverified in every doc.
2. **Check.** `rusty_orch lens check /srv/lel/lens-local.json --assertions /etc/let-em-loose/assertions.json`. Pass: exit 0 and `local`, `local-high`, `local-rebel` admitted. The assertions file is not needed for local providers; Claude and Codex are not in this config.
3. **Open.** `bbp open --mode claims --dir /srv/lel/bbp --task smoke-1 --repo /srv/lel/toy/repo --brief brief.md --roster roster.json --profiles profiles.json --holdout-profiles holdout-profiles.json --holdout-root /srv/lel/toy/holdout`. The roster has principals `seed` (post claim), `architect` (post claim), `adversary`, `operator` (post evidence), read `All`, `write` empty; no roster file for claims exists in the repo, so it is written for this run (pattern: `rusty_bbp/tests/claims.rs` `lens()`). `bbp assign --role X --principal X --vendor local` for each (`vendor` is free text).
4. **Link** the task to a card per #1092 verification step 4 so `inbox_list` shows it.
5. **Baseline reads.** `bbp card`, `bbp verify --dir /srv/lel/bbp --task smoke-1` (head hash recorded in the run record).
6. **Seed** Q and A through a hand-driven `bbp mcp` session as `seed` (`BBP_SESSION=yes`, `BBP_PRINCIPAL=seed`): one JSON-RPC `post` call each, `kind: claim`, shapes in `post_args` (`rusty_orch/src/lens/bbp.rs`) and the claim table in the addendum. There is no `bbp` verb or orch command that posts a question; see the Drift and ESCALATE 2.
7. **Run.** `rusty_orch lens run /srv/lel/lens-local.json --bbp /opt/lel/current/bin/bbp --dir /srv/lel/bbp --task smoke-1 --state /srv/lel/orch-state --experiment smoke-1 --max-passes 8`. Stdout is the record; keep it.
8. **Read back.** `bbp card`; `bbp why --dir /srv/lel/bbp --task smoke-1 --claim <A>`; same for one candidate; `bbp inbox --dir /srv/lel/bbp --task smoke-1` and the board's `inbox_list`; `rusty_orch lens record --bbp ... --dir ... --task smoke-1 --state ...`; `bbp verify` again (chain intact, head moved).
9. **Act.** At the inbox, Nano (or the operator for Nano, with his word) taps Reject on A (`bbp human claim <A> drop` via the board). Nothing is deleted at any point: nothing in M1 acts on an accepted claim. Also settle Q with `choose` if it escalates `stuck`.
10. **Record.** Write `docs/projects/let-em-loose/1166-smoke-run-record.md` in the form of `proving-run.md` section 5: outcome, calls and tokens per lens (first measured figures), refusals by code, each lens's posts, human actions, surprises. Do not raise a limit to make it pass.

## 9. Pass criteria
Run-level (card may word these differently; reconcile when the card is read):
1. `lens check` exits 0 and the open succeeds with the holdout frozen (step 2, 3).
2. `lens run` ends by no job started or `--max-passes`, not by `task_calls`; at least 3 candidate claims on Q from 3 distinct providers.
3. At least one claim reaches accept (8) by two distinct authors' non-opinion support, or drops; Q ends accepted, escalated or chosen, with a recorded reason.
4. **The inbox fires:** A reaches the strict line (12) and escalates `irreversible`, or escalates `budget` or `stuck`; it shows in both `bbp inbox` and `inbox_list`; the human action lands as an event; A is never `claim_accepted` without a human decision (CL-12).
5. `bbp why --claim` prints a history for A and a candidate with the rule that fired; `lens record` prints one line per lens; `bbp verify` passes before and after.
6. Calls stayed within `task_calls 60`; no provider outside the local three was called; no nonzero price anywhere.
7. Refusals are listed, not hidden (section 10).
**Not in this run, deferred:** accept waits for a holdout pass (needs a candidate, #600); any breaker criterion (#758); Claude and Codex lens behaviour (D134).
If the models never push A to 12 (they may post nothing, opinion only, or malformed JSON), criterion 4 is not met by the lenses. That is a finding, recorded. The inbox mechanics are separately proven by #1092 verification step 4.

## 10. The executed-evidence gap and this run
- orch's `post::parse` accepts `evidence_kind: executed` from a lens (strength 1, one cited reference; `post.rs`), and its own reply protocol text (`post::protocol`) lists `executed` as an allowed kind. The 1123 prompts forbid it (shared rule 4) but the prompt is the only guard on the orch side.
- The host is the real guard. `check_executed` in `rusty_bbp/src/claims_engine.rs` refuses an `executed` post unless it cites a final test report written by the runner for a run whose candidate names that same claim: `evidence_required` (no artifact), `evidence_unbound` (not a report, not the runner's, another claim's candidate, or an error report) or `not_passed`. This settles 1123's "Open" bullet from code: a lens cannot post `executed` evidence that counts, and nothing is scored from one.
- **For this run:** no candidate and no runner report can exist, so every `executed` post from a local lens is refused. Expect some from small models, since the protocol text offers the kind. Consequences: a wasted call that counts as refused (it is not free, `round.rs`), a `refused` count on that lens's record line, no score movement, no budget unit spent (the post is not committed). It cannot make the run falsely pass.
- **Residual:** the orch-side `parse` still accepts it, so a malicious or buggy host build could admit it. Fixing `parse` (reject `executed` from a lens at the source) is a code change to orch, not in M1's freeze; recorded as a follow-up, not done. Criterion 7 reports the count of refusals and their codes.

## Defaults taken
- [default] Local-only config as a separate `lens-local.json`; no flag or code change.
- [default] Research lens omitted; `local-rebel` is Qwen under a third id.
- [default] Toy is a shell tool `sweep`; irreversible claim A seeded by the operator, so the inbox does not depend on a model choosing `reversible: false` (a model may still do so on its own claims).
- [default] `"erosion": "off"` in the toy profile set.
- [default] `task_calls 60`, `--max-passes 8`, 90-minute time box; roster written for this run.
- [default] The run is read-only for the world: no action on an accepted claim.

## Drift
- No verb or command posts the question (domain `question`) in a live task; `lens run` assumes it exists. Only the architect lens posts claims, and only as answers. Orch's tests build it by hand.
- No claims-mode roster file is in the repo (only `rusty_bbp/tests/claims.rs`); `docs/proving/roster.json` is the four-role stage-mode roster.
- 1123 fixture and the llama-swap port 8080 and model ids are unverified; `STARTER` has `baileyai` as the host.
- 1124 says the smoke-run paid half is deferred (D134); this design supersedes the card's "local model first, then paid" order: only the first half runs.
- The card's "Waits on #758" is wrong for the first run (section 7). The card text itself was not read.
- Card lists `bbp why --claim` and `bbp verify`: both exist (`bbp.rs`). `lens why` (D58) is not used.

## ESCALATE (Nano)
1. ESCALATE: Is it acceptable that this first smoke run does not exercise the holdout gate (an accept waiting for a holdout pass), because that needs a candidate and M1 has no builder?
   OPTIONS: a) yes, stage and freeze the holdout now, test the gate in the first #600 builder run. b) also store one hand-made candidate on a claim (new operator step and a holdout lens role; heavier, edges toward new design under D127). c) drop the holdout from this run.
   RECOMMEND: a) it keeps the freeze (D127) and the holdout is still validated at open.
   REVERSES: none.
2. ESCALATE: How should the question claim be posted in this run: by a hand-driven `bbp mcp` session now, or by a new `rusty_orch lens seed` command first?
   OPTIONS: a) hand-driven session; no code; a documented one-off. b) a new read-write `lens seed` command (public API, ask-first, D127 freeze, and delays the run). c) the architect lens posts the question itself (changes the proposer rule; new design).
   RECOMMEND: a) it is reversible, adds no surface, and the Drift records the missing verb for later.
   REVERSES: none.

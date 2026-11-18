# #1166 Smoke run: lens run on a toy claims task, local model first (step 1 design)

Status: design for Nano's approval (card #1166 step 1). Nothing is built, deployed or run. No model was called and the board was not read (the card text was not available to the author; pass criteria below follow the brief for this design and are marked where the card may differ).
Authority: D130 (M1 design batch), D131 (deploy on BaileyAi), **D134 (no paid runs for now: this run is local-only; D141 on 2026-11-17 confirms this run stays local-only; the paid run is run 2, the launcher brief)**, D127 (freeze: nothing new), D53, D86, D93, D37.
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

`local` = Qwen3.6-35B-A3B, `local-high` = gpt-oss-120b, `local-rebel` = Qwen3.6-35B-A3B again under its own id (same weights; the rebel differs only by orch's rebel suffix, so the three candidates are not three independent models: noted in the record, not hidden). Adversary leads local-high and operator leads local, so `check_diversity` passes. All three are `openai-http` with `max_class` sensitive by default, price 0, `context_tokens` 32768, `base_url http://<llama-swap-host>:<port>/v1` and ids `qwen3.6-35b-a3b` and `gpt-oss-120b` (checked on BaileyAi 2026-11-17: llama-swap does not answer on loopback and routes by exact, lowercase id; the config is `lens-local.json` in baileyrd/Factory, Factory#2). **Research is omitted [default]:** its `tools: read-only` needs a CLI provider (`config.rs`) and without tools it has nothing it may cite, so it would post nothing.
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
- **For this run:** no candidate and no runner report can exist, so every `executed` post from a local lens is refused. Expect some from small models, since the protocol text offers the kind. A refused post costs a call and is not scored. **Correction after run 1 (2026-11-17):** this section predicted a refused post would be a non-fatal `refused` count. It is not: orch halts the run on any board-refused post (`round.rs` `partial` -> `Halt::Board`, exit 1), which is what ended run 1 when a `local-high` reply posted `executed`. D144 rejects `executed` at parse; whether any refused post should halt is Q51.
- **Residual:** the orch-side `parse` accepted it, so run 1 halted on it. Rejecting `executed` at `parse` is a bug fix (D144), to be done before the rerun. Criterion 7 reports the count of refusals and their codes.

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
- 1124 says the smoke-run paid half is deferred (D134); this design supersedes the card's "local model first, then paid" order: only the first half runs. Update 2026-11-17: D141 (as amended by Nano's correction) keeps this run local-only: `--max-passes 8`, `task_calls 60`, $0, `local`, `local-high`, `local-rebel`, no paid provider. The paid run is run 2, the SSH host launcher brief, after this run has finished and been read: `--max-passes 5`, default `task_calls` cap (400), no dollar-cap enforcement in v0, codex paid now and claude only after its `no_overage` entry exists in `/etc/let-em-loose/assertions.json`. Both wait on #1092 (deploy) and #1123's `lens.json` on the host.
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

## Run 1 result (2026-11-17, task `smoke-1`, local-only, $0)
Not a clean pass (D143: the first run is a clean #1166 pass, so this does not count). Detail: remind-me `mem_ddc10f27ebb84572b6bcec43e05d21dc`; PR #1779.
- Machinery ran end to end (deploy, `bbp open`, seed, `lens run`, read-back); `bbp verify` chain intact (40 lines, 0 unchained); no AVC denial (D142).
- Criterion 2 unmet: 2 candidate claims, not 3 from 3 providers. Criterion 3 unmet: A was dropped by DropLine at -8 (two counter authors), no claim accepted. Criterion 4 unmet: the inbox stayed empty. Criterion 1 and 5's `bbp verify` met; criterion 6 and the refusal listing of criterion 7 not stated here.
- Defects: `executed` posted by a lens halted the run (D144, Q51); gpt-oss-120b via llama.cpp build f74e2fa returned HTTP 500 twice on valid harmony output (D145); the run process died on a closed stdout and was resumed; the seed did not exercise the inbox (D146). Rerun after the fixes (Q52).
- Host facts: llama-swap answers only at `<llama-swap-host>:<port>` with lowercase model ids (`qwen3.6-35b-a3b`).

## Run 2 result (2026-11-17, task `smoke-2`, local-only, ~$0)
After PRs #1779, #1781, #1782 and #1783 merged and redeployed. Exit 0, `ended: Quiet`, no AVC denials, `bbp verify` clean (7 to 31 lines, 0 unchained). Evidence on BaileyAi in `~/smoke-2` (`run.out`, `why-A.json`, `why-A2.json`, `inbox.json`, `lens-record.txt`). Claims: Q=1, A=2, A2=3 (fixture), 4 and 5 fixture supports, candidates 6 (`local`) and 7 (`local-rebel`), 8 operator evidence on A, 9 adversary evidence on A.
Against section 9: 1 met. 2 not met (2 candidates from 2 providers: `local-high` failed Propose, D149). 3 not met (A dropped, but Q stayed Open and no candidate was reviewed, D148). 4 mechanics met: the inbox fired on A2 (fixture, not lens-driven, D146), and Nano reports that he dropped A2 with `bbp human ... claim 3 drop`, that the board answered "Ok", and that `bbp inbox --task smoke-2` now returns `[]`. So the human action on the inbox escalation landed. 5 `why --claim` for A and A2 printed, no candidate `why` yet, `lens record` 2 lines, `bbp verify` clean before and after. 6 within the call cap (about 6 calls, only the three local providers). 7 no board refusals (0 rejections). Not verified: candidate `why`, the board `inbox_list` and the card link (the coordinator cannot read the store, so the A2 drop rests on Nano's report).
**Verdict (D143):** not yet the first run; the freeze holds.
Findings:
- Third candidate missing: `local-high` replied with an `evidence` field on a claim; parse refused it (D149).
- Candidates never reviewed: stale question view in the lens seat (D148).
- Mislabelled direction: msg 9 (adversary, `local-high`) is Counter / Cited / strength 2 (-4) citing art:1, but its note argues the action is safe; msg 8 (operator) cites msg 2, the claim itself. With both, A reached -8 and DropLine dropped it (Q53). Evidence: `why-A.json`, `lens-record.txt`.
- Wrapper bugs (CDPATH made `cd` print the directory twice; the wrapper did not require or pass `LLAMA_SWAP`) are fixed in PR #1788.

## Run 3 result (2026-11-17, task `smoke-3`, local-only, $0)
After #1788, #1790 and Factory#3 deployed (Factory at 831268c). Exit 0, `ended: Quiet`, 4 passes, about 17 lens calls, only `local`, `local-high` and `local-rebel`, board rejections 0, no AVC denials, no HTTP 500 from llama-swap during the run, `bbp verify` clean (7 to 44 lines, 0 unchained). Evidence on BaileyAi in `~/smoke-3`. Ids: Q=1, A=2, A2=3 (fixture), 4 and 5 fixture supports, candidates 6 (`local`), 8 (`local-rebel`, Propose(1) in pass 2), 9 (`local-high` as Rebel in pass 2), evidence 7 and 10 to 16. Message ids 10 to 15 and the drops are verified by Nano's `bbp why --dir /srv/lel/bbp --task smoke-3 --claim 1,6,8,9` output: Q (claim 1) is Open, score 0, only Posted, no events; claim 6 was Dropped by DropLine at rev 78 (msg 10 adversary refs Msg 1, the question; msg 13 operator refs Msg 6, itself), claim 8 at rev 82 (msg 11 adversary refs Art 1, the brief; msg 14 operator refs Msg 3, the A2 fixture), claim 9 at rev 92 (msg 12 adversary refs Msg 3; msg 15 operator refs Msg 9, itself). Each drop is two cited counters of -4. Still unverified: the board `inbox_list` and the card link.
Against section 9: 1 met. 2 met (3 candidates from 3 distinct providers; the pass-2 fallback absorbed a failed Propose(1) and a failed Rebel). **3 NOT met:** every candidate and A were dropped by DropLine (two cited counters of -4 each), so Q is Open with nothing accepted, escalated or chosen. 4 met by mechanics through fixture A2 (`inbox.json` shows only A2). 5 met for `why` on Q, 6, 8, 9 (Nano's output above) plus A and A2 and `lens record`; `inbox_list` and the card link are not verified. 6 met (about 17 calls, local only). 7 met (0 refusals).
**Verdict (D143):** not yet the first run. The cause is behavioural: reviewers only attack (Q54). Nano's answer (D151): reviewer prompts also post support, only when they can point to text in the brief that the claim satisfies, and nothing when unsure; no host change (D152).
**Rerun plan (smoke-4, D152):** after one Factory PR with D150 (architect: "Reply with one claim post and no evidence post.") and the reviewer wording in `lenses/adversary.md` and `lenses/operator.md` (D151), on top of D148 (orch re-read) and D149. Pass criteria are section 9 unchanged.
Findings: reviewers counter everything (Q54, resolved by D151 for now); counter notes cite things that do not exist (Q53 additions); an evidence post beside a proposal refused the whole reply (D150); three empty Qwen replies at the 4096-token cap and one malformed gpt-oss JSON reply (reasoning budget, held under D147/D152); the lens never sees the brief's artifact id (Q55).

## Run 4 result (2026-11-17, task `smoke-4`, local-only, $0)
After D148 (orch re-read) and Factory#3 and #4 (prompt fixes) deployed. Exit 0, `ended: Quiet`, 6 passes, about 18 lens calls, only `local`, `local-high` and `local-rebel`, board rejections 0, no AVC denials, `bbp verify` clean (7 to 50 lines, 0 unchained). Evidence on BaileyAi in `~/smoke-4` (`run.out`, `why-A.json`, `inbox.json`, `lens-record.txt`). Ids from `inbox.json` and `run.out` order: Q=1, A=2, A2=3 (fixture), 4 and 5 fixture supports; candidate 6 (`local-high`, Propose(1), `reversible: false`, stakes low); 7 operator Counter on A; candidate 8 (`local-rebel`, Propose(0) in pass 2, `reversible: false`, deps [msg 3]); 9 adversary Counter on A (A dropped at -8); 10 adversary Support on 6 (cites art:1); 11 adversary Counter on 8; 12 operator Support on 6 (cites Msg 3); 13 operator Counter on 8 (cites Msg 3); 14 operator Counter on 6 (look-harder, cites Msg 6, itself); 15 adversary Support on 6 (art:1); 16 operator Support strength 3 (art:1).
Against section 9: 1 met. 2 NOT met: 2 candidates from 2 providers (`local-high`, `local-rebel`). In pass 1 Propose(0) on `local` and the rebel on `local-rebel` returned empty content (`reply is not JSON: unexpected end of input`); Propose(1) on `local-high` landed. In pass 2 Propose(0) retried on `local-rebel` and landed, but the rebel retried on `local` and was empty again, so `rebel_failures` hit `REBEL_TRIES=2` and the rebel slot ended with no candidate. 3 pending: candidate 6 is escalated and awaits Nano's human decision; Q itself stays Open until a `choose` or an accept. 4 met on a lens-driven escalation (the fixture A2 also fired). 5 `why` printed per the run files; `inbox_list` and the card link not verified. 6 met (about 18 calls, local only). 7 met (0 refusals; 2 gpt-oss HTTP 500 `peg-native` transients, in pass 4 and pass 5, both absorbed; 3 empty Qwen replies).
**First lens-driven escalation, and what it rests on.** Claim 6 scored +4 (10), +4 (12), -4 (14), +4 (15), +6 (16) = 14 against `accept_at_strict` 12 (strict because `reversible: false`), so it escalated `irreversible` (`smoke-4/msg:6/112`). Msg 12 is an invented run: citing Msg 3, the A2 claim ("Running `sweep --yes` ... unlinked the files as specified"). Without it the score is 10 and nothing escalates (Q53).
**Verdict (D143):** not yet the first run: criterion 2 unmet, criterion 3 pending Nano's action on claim 6, and the escalation rests on an invented citation. The prompt route (D151) worked: reviewers now post support.
What Nano's action does (addendum C-R7, C-R8, CL-55; the coordinator read `claims_engine.rs`): `bbp human ... claim 6 accept` sets claim 6 Accepted, with `sole_candidate` true if fewer than two candidates are live; `choose 1 6` accepts 6 and drops the other live candidates (8); neither changes Q's own status. "Q ends chosen" in 9.3 refers to the human `choose` on the question.
Findings: invented citations decide outcomes (Q53); one author scores up to three items on a claim (Q56); a design claim marked irreversible (Q57); the empty Qwen replies recurred and cost the third candidate (D152 condition); two gpt-oss 500s absorbed.
**Grounding audit of claim 6 (2026-11-18; Nano's request, D155).** Brief art:1 has 9 lines (line 5: files directly inside DIR "whose modification time is more than DAYS days old"; line 7: never follow a symlink, touch a subdirectory, act on a bad DAYS, or delete without `--yes`); it has no "floor", "86400" or "stat". Msg 10 grounded in lines 5 to 7 (its closing "floor division by 86400" is its own inference). Msg 12 ungrounded (cites the A2 claim as a run). Msg 14 not brief-grounded (the claim's silence on unlink errors; self-cited). Msgs 15 and 16 ungrounded (attribute the floor and a non-followed stat to the brief). Scores: board 14; grounded only with repeats 0 (+4 without msg 14); one vote per author: latest (15, 16) 10, first (10, 12) 8; one grounded support author against `min_authors` 2. Floor deviation (Q58): claim 6 keeps files aged between DAYS and DAYS+1 days that line 5 says to delete. **Nano dropped claim 6 (D154); Q on smoke-4 has no accepted or chosen candidate.** Full detail in Q53.

## Run 5 result (2026-11-18, task `smoke-5`, local-only, $0)
After D153's `--reasoning-budget 2048` on the Qwen entry (`~/llama/config.yaml` line 17); everything else as smoke-4. Exit 0, `ended: Quiet`, 6 passes, about 20 lens calls, board rejections 0, `bbp verify` clean (7 to 53 lines, 0 unchained). Evidence on BaileyAi in `~/smoke-5` (`run.out`, `why-A.json`, `inbox.json`, `grounding.md`).
**The reasoning budget worked:** 0 empty Qwen replies (3 in smoke-3, 3 in smoke-4). The llama.cpp rebuild (D145) stays held: 2 gpt-oss HTTP 500 `peg-native` transients, both absorbed.
Candidates: msg 6 (`local-high`, Propose(1)) and msg 7 (`local-rebel`, Rebel). Propose(0) on `local` failed Permanent (`post 0: a proposal must answer msg:1`), and pass 2 skipped the slot, so criterion 2 is unmet again (2 candidates from 2 providers; D156). Both candidates were Dropped: claim 6 at -10 (+6 -4 -4 -4 -4, dropped at msg 19) and claim 7 at -10 (-2 opinion, -4, -4, dropped at msg 14; msgs 15 and 16 landed after the drop, non-scoring), derived by the coordinator (board `why` for 6 and 7 not captured). A dropped at -8 (rev 60, msgs 8 and 9). Q is Open with no live candidate; criterion 3 unmet. The only escalation is the fixture A2 (criterion 4 by the fixture only).
**Grounding audit (D155):** the escalated claim is only A2; its two votes (fixture supports 4 and 5) are explicit fixtures with no lens judgement, so there is nothing to ground and it is safe to drop. For the dropped candidates, most counters are grounded (msgs 8, 13, 18, 19 quote brief line 7; msg 12 is true); the exceptions are in Q53 (msgs 10, 11, 14, 15, 16, 17).
**Verdict (D143):** not yet the first run. The failure is no longer an artefact of attack-only reviewers: both candidates had real flaws. Whether the bar is reachable, and what "first run" must mean when lenses legitimately drop every candidate, is escalated to Nano.


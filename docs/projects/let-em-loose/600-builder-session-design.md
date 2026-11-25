# #600 builder sessions: design (draft for Nano's approval)

> Approved as a whole on 2026-11-04 (D66). Copied unchanged into the repo on
> 2026-11-25 from `/mnt/project-files/let-em-loose/`, where OPEN_QUESTIONS Q44
> thought it lost (D94: designs live here). `600-build-plan.md` follows it;
> D177 and D178 record what it settles for #1404 and #1405.

Card #600, "Run the dedup experiment (prototypes included)". This closes OPEN_QUESTIONS Q7 once it is approved.
Author: Claude, 2026-11-04. Status: **draft, no code until approved** (D25).
Builds on: D53 (one host, BaileyAi), D56 (Windows shadow run + golden), D57 (#596 erosion rule), the claims addendum (§ Runs and candidates, Failing runs block, Budget), #594 lens runner §9, and #597 ground truth.

## 0. Summary

A builder session is one Claude CLI process that turns one design claim into one candidate. It runs in its own `rusty_worktree` checkout inside `rusty_sandbox`, on BaileyAi, launched through orch-bbp's leased, confined path.

- **What it sees:** INTENT.md, its claim, the claim's dependencies and its own worktree. It sees nothing else.
- **What it produces:** diffs and a `candidate {base, diffs, claim}`. Storing the candidate debits the claim's budget and starts a sandboxed run. The runner's `test_report` is the only route to `executed` evidence.
- **What it can't do:** post evidence, settle anything, see holdout tests, or write protected paths.

Orch schedules builders the way it schedules lenses. Escalations go to the inbox (#595). No new BBP schema is needed.

## 1. The experiment repo

- **Repo:** a fresh, empty git repo `dedup-manager` on BaileyAi, owned by `orch-runner`. It is local only, with no remote in v0 (logged default, §10).
- **First commit:** INTENT.md, intent v0.1-stage1 exactly as frozen on card #600 (2026-11-02). This commit is the `base` of every first-generation candidate.
- **Ground truth sits outside the repo:**
  - The holdout suite lives in orch-store/host config, which builders can't read (#597).
  - The golden sets live in the Factory repo at `golden/dedup-manager/`. Nano signs them off (Q4).
- **Protected paths:** `tests/frozen/**` and `golden/**` are frozen at task open (#597 `protected`).

## 2. Task open

One claims-mode BBP task. Its frozen profile set is:

| Profile | Source | Runs where |
| --- | --- | --- |
| `suite:*` | Frozen tests written by the Survey/Intent lens from the intent alone, before any candidate exists | Linux sandbox |
| `holdout:*`, including `golden:*` | A holdout lens on a model no builder uses (D37). The Nano-signed golden cases (Q4) are `golden:` profiles inside this suite | Linux sandbox. Only the holdout lens's holdout candidate runs this suite; builder candidates never run it (addendum § Holdout suites) |
| `erosion` | `rusty_erosion` against the pinned base, report-only until calibrated (D41, D42) | Linux sandbox |
| `perf` | First scan ≤ 30 min/TB and incremental rescan < 1 h after adding 50 GB, on a 500 GB synthetic tree, scaled | Linux sandbox |

The Windows shadow run (D56) is **not** in the frozen profile set. It runs read-only on Nano's Windows machine, Nano starts it, it never debits a budget, and its result reaches the claim as described in §7.

The baseline suite must be green at open, or the open is refused. This uses `bbp open --base`/`baseline_red` (#598 §6, pending approval). Each Windows-specific intent line (mapped drives, UNC paths, Windows first) must map to a `golden:` case or to the shadow run (D56). **New orch check (needs approval, §8):** at open, orch lists any such line with no mapping as `untested`. No host check does this today.

## 3. Who writes what

| Step | Actor | Writes |
| --- | --- | --- |
| Seed | Survey/Intent lens (#598) | One claim per intent `always`/`never`/`example`/target line, tagged with domain and reversibility |
| Propose | Architect: 3 blind proposers on 3 models, one of them the rebel (#594 §9) | Design claims, each with its done-when as dependent claims |
| Build | **Builder session** (this card) | `diff` and `candidate` for one design claim |
| Run | Host runner in `rusty_sandbox` | `test_report` and `log` (run secret) |
| Judge | Research, Adversary, Operator (rotated, never the claim's author) | `executed` and `cited` evidence |
| Decide | Engine | Accept or drop by thresholds, with escalations going to the inbox |
| Human | Nano | `accept`/`choose` (C-R8), budget Approve, reject, refine |

A builder is a role with `write ∋ {diff, candidate}` and no `evidence` write. It can't support its own work, which keeps "the reviewer is never the builder" mechanical rather than a prompt rule.

## 4. One builder session

1. **Admission.** Orch picks an `open` design claim that has no live candidate, or whose latest candidate failed. It checks `spent(c) < budget(c)` by reading the task.
2. **Worktree.** `rusty_worktree` creates `wt/<claim>/<n>` from the claim's base: the INTENT commit, or the accepted dependency's tree. The worktree is the session's only writable path.
3. **Launch.** orch-bbp launches Claude CLI under a permission lease:
   - writable: the worktree;
   - readable: INTENT.md, the claim and its dependencies (through `claim_card`), and the profile names with their pass/fail counts;
   - network: none, apart from the Claude subscription endpoint the CLI needs. Dependencies come from a crate/package mirror pre-fetched into the sandbox at task open. This is `[default]`, and the mirror's contents are fixed in the task's frozen inputs;
   - limits: no paid calls (#594 zero-rate admission) and a wall clock of 45 min per session (config).
4. **Work.** The builder edits, builds and runs the visible `suite:*` tests in the worktree as often as it likes. These local runs aren't evidence and cost no budget.
5. **Submit.** The builder posts its `diff`s and one `candidate {base, diffs, claim}`. The host refuses the candidate (`protected_path`) if it touches a protected path. If it's accepted, `run_started` and a debit happen in the same transaction.
6. **Run.** The runner executes the frozen build profiles (not the holdout suite) on the candidate tree in a fresh sandbox and writes the final `test_report`. A failed `erosion` or other profile blocks acceptance until a later passing run, or until Nano accepts or chooses explicitly (C-R8). The holdout lens separately runs the holdout suite against the current candidate tree (D37).
7. **End.** The session ends when the candidate is stored, the wall clock runs out (with no candidate stored and no debit), or the builder gives up and posts nothing. The worktree is kept until the claim settles, then pruned.

**Retries.** A failed run is visible to the next builder on the same claim, which gets the failing profile names and the failed counts. A new candidate costs one more budget unit. When the budget is spent, the claim escalates to the inbox (budget), and Approve adds one base allowance (#595).

## 5. Competing prototypes

- **Per design claim:** up to `builders_per_claim` concurrent sessions. The default is 1, because the Architect has already produced 3 competing designs.
- **Per task:** at most `max_live_builders` sessions at once. The default is 3, one per design. Both are config values covered by the task's frozen inputs.
- **Builder model:** Claude CLI only in v0, as card #600's scope says. This is open question O2.
- **Taste:** stage 1 has no `taste` lines, so there are no `choose` escalations for taste.
- **Ties:** two accepted designs that do the same job follow #596's tie rule: keep the simpler or cheaper branch and record why (mem_f4df31cb…). Fork-and-collapse starts only after #600 (D36), so it doesn't apply here.

## 6. Measuring (for #596)

Every session records the following to orch-store:
- claim and candidate ids;
- tokens in and out, and wall time;
- the run result for each profile;
- the erosion delta;
- whether the claim was later accepted, reversed or reopened.

#596 compares arms on these numbers: holdout no worse on 5 of 5 runs, tokens at most 80% on 4 of 5, escalations no more than baseline, and erosion no worse on 4 of 5 (D57). Nothing here adds a new metric.

## 7. Failure handling

| Failure | Behaviour |
| --- | --- |
| Sandbox or worktree setup fails | The session isn't started and nothing is debited. Orch retries once. A second failure posts a host problem to the inbox |
| CLI crashes or the lease ends mid-session | No candidate, no debit, and the worktree is kept for inspection. The next admission starts a fresh worktree |
| Candidate refused (protected path, budget, envelope) | The refusal reason goes into the next session's context. Nothing is debited |
| Run times out | The runner writes an `error` report. Per addendum § Failing runs block, an error doesn't change `failing(c)`, so the claim doesn't settle on it. Orch schedules one rerun; if that also errors, it goes to the inbox as a host problem |
| Windows shadow result contradicts a Linux pass | It can't be `executed` evidence, because executed evidence must cite the claim's own runner report (C-R4). Nano reopens the claim by a human `decision reopen` (C-R8, after `budget_extended` if spent), or posts the result as `cited` evidence, which doesn't reopen. O3 asks which |

## 8. Changes that need approval

1. **orch:**
   - the `Builder` job kind in the scheduler and its admission check (§4.1);
   - the config values `builders_per_claim`, `max_live_builders` and `builder_wall_secs`;
   - an orch-store `builder_session` table (§6).
   - the open-time `untested` mapping check for Windows intent lines (§2).
2. **orch-bbp:** a builder launch profile under the existing lease mechanism, with the pre-fetched dependency mirror.
3. **Depends on the rest of #598 §6 (decision card pending):**
   - named sandbox profiles for builders;
   - the Survey and Intent lens kinds;
   - the `shadow:` profile kind;
   - `bbp open --base` / `baseline_red`.

   This design can't be built until those are approved. Beyond them and #597 `protected`, the host needs nothing new.
4. **BBP:** none.
5. **New repo:** `dedup-manager` on BaileyAi, local only.

## 9. Tests (to write with the code)

- **Admission:**
  - a claim with a live candidate gets no second builder;
  - a spent budget gets none;
  - `max_live_builders` is respected.
- **Confinement:**
  - a builder writing outside its worktree fails;
  - a builder can't read the holdout suite;
  - a builder's attempt to post evidence is refused by role.
- **Debit:**
  - a crash before submission debits nothing;
  - a stored candidate debits exactly once, including on replay.
- **Protected paths:** a candidate that adds `tests/frozen/x` is refused with the path named.
- **Retry context:** the next session sees the previous failed profile names, not the holdout test bodies.
- **End to end (real host, scripted CLI):**
  1. intent → seed → 3 designs → 3 builders → runs → one accepted claim;
  2. a builder that pastes duplicate code gets an erosion report while report-only, and is blocked once enforcing.

## 10. Logged defaults (change them if you disagree)

- The experiment repo is local-only on BaileyAi (D53) and can be pushed later.
- The builder wall clock is 45 minutes per session. It's config.
- Dependencies come from a mirror pre-fetched at task open (§4.3).

## 11. Open questions for Nano

- **O1.** The intent's desktop GUI is Windows-first, but claims runs are Linux-only. Should the GUI be judged only through the Windows shadow run and golden cases, as D56 says, with Linux checking only the engine and the web UI? *Recommend yes.*
- **O2.** Should builders be Claude CLI only in v0, as card #600's scope says, or should Codex in `lel-codex` also build? *Recommend Claude-only*, so the holdout lens keeps a model that no builder uses (D37) without leaving only local models.
- **O3.** How does a Windows shadow result that contradicts a Linux pass reach the claim? Options: (a) you reopen the claim with a human decision, (b) it is posted as cited evidence, which doesn't reopen. *Recommend (a)*: a real-data failure should stop acceptance, and only a human reopen can do that.

## Owner answers (2026-11-04)

Nano, thread message `cmsg_01WgyiM31RsSnZTgNuPX2VE4MwQzj2Aawhe7Q5axWeGqTY` (17:16Z): "01 Yes. 02. Concur. 03. Concur with recommendation".
- **O1: yes.** The Windows GUI is judged only through the Windows shadow run and golden cases. Linux checks the engine and the web UI.
- **O2: yes.** Builders are Claude CLI only in v0.
- **O3: (a).** A Windows shadow result that contradicts a Linux pass reaches the claim through Nano's human `decision reopen` (C-R8).

The §10 defaults were shown to Nano and he didn't object. They stay `[default]`. Approval of the design as a whole (D25) is still to be confirmed.

Design approved as a whole: Nano, decision card `cmsg_01WgyiM31RsSnZTgNuPX2VE4UkTT9YFWDhx1CsEYPh6YFg`, option A, 2026-11-04T18:08:52Z (recorded as D66).

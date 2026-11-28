# #1236 design: the dedup stage-1 holdout suite and protected test paths

Status: **design for Nano's review**, 2026-11-28 (card #1236, epic #600). Nothing
is built, generated or frozen until Nano approves this on the card (D25).
Inputs: INTENT.md v0.1-stage1 (sha256 `79f66e7d…`, repo `dedup-manager`, HEAD
`c0c83a95`, D179), D18, D37, D56, D63–D66, D134, D178, the #597 ground-truth
design, `600-builder-session-design.md` (D66), `600-build-plan.md`, and
`rusty_bbp_host` `holdout.rs`, `golden.rs`, `profiles.rs`, `protect.rs` as on main.

The let-em-loose-expert answered six design points on 2026-11-28. Three go to
Nano (section 9: the test seam, the toolchain and the holdout model). Two are
logged defaults: D228 (leak review) and D229 (freezing). One had no conflict
(suite shape). Sections 2 to 7 show the recommended option for each open point;
if Nano picks another option, only that section changes.

## 1. What it is

A second frozen profile set for the stage-1 claims task (D37): tests written from
the intent alone by a holdout lens on a model no builder uses. Only the holdout
lens's holdout candidate runs it, on the same base and diffs as the claim's current
candidate. Builders see profile names and pass or fail counts, never the files or
full logs. A claim with a candidate can't be accepted automatically without a
passed holdout run on its current tree (G1). Protected globs make the visible
frozen tests and the golden set unchangeable by builder candidates (G2).

## 2. The gap this design has to close: a test seam

INTENT.md names no testable interface: "one scanning engine; a desktop GUI and a
local web UI over the same engine". The #597 design says the holdout lens writes
"against the interface the intent states", and golden cases (#1238) run a `cmd` in
the candidate tree. Both need something to call, and builders, who never see the
holdouts, have to know what it is.

**Recommended (Q1 b): a new intent version, `0.2-stage1`, that adds one
interface line and writes the contract out in the same INTENT.md.** Builders still
get only the intent (D18), the words are Nano's, and no task is open, so revising
now costs nothing. It changes D179 (the repo's one file and its 1872 bytes), so
Nano decides.

The contract, kept implementation-neutral:

- An executable `seam/run` in the candidate tree. It may build the app on first call.
- `seam/run scan --state DIR ROOT...` prints one JSON object: `version: 1` and
  `groups`, each with `id`, `kind` (`bytes` or `image`), `files` (`path`, `size`),
  `keeper` and `keeper_reason`.
- `seam/run quarantine --state DIR --into Q --confirm ID...` moves every non-keeper
  of the named groups into Q. Without `--confirm` nothing moves.
- `seam/run purge --into Q --confirm` empties Q.
- Exit 0 on success, 2 on a refused request (such as confirming a last copy),
  anything else is a failure.

The visible `suite:*` tests, the holdout tests and the golden cases (#1238) call
only this. It says nothing about how the engine, GUI or web UI are built.

Rejected alternatives: (a) a separate `tests/frozen/SEAM.md` committed as a second
commit, which gives builders a second input beside the intent and stretches D18;
(c) testing through the web UI's HTTP API, which is just as unspecified.

## 3. Toolchain

Holdout profiles freeze their `read_roots` and environment at open, and the
approved dependency mirror (D66) holds "the dependencies INTENT.md's toolchain
needs", but no language is fixed anywhere. **Decided (D244, Nano 2026-11-28): Rust stable, pinned**, stated in the same intent
revision as Q1 if Q1 revises the intent, otherwise wherever the seam contract lives. Erosion is measured for Rust
only (D45); other code is `unmeasured`, which would leave #596's "erosion no worse
on 4 of 5" (D57) unreadable, and the control `rusty_fclone` is Rust (D18). The
tests stay language-neutral regardless: they call only `seam/run`.

## 4. Who writes the suite

**Decided (D245, Nano 2026-11-28): gpt-oss-120b via llama-swap on BaileyAi, with
Qwen3.6-35B-A3B as fallback.** No builder uses either (builders are Claude CLI
only, D64), and both are local and free, so D134 (no paid runs) holds. Known risk:
the llama.cpp build returns HTTP 500 on some valid gpt-oss output (D145) while its
rebuild is held (D147); the generator retries, and falls back to Qwen if gpt-oss
keeps failing (then gpt-oss reviews, D228). Codex is out: it counts as paid under
D134, and D64 keeps it off builders so the holdout lens has a free model.

The author gets a frozen prompt holding INTENT.md, the profile names in section 5
and the style rules there. It does not get this design's coverage table: the cases
must be the holdout model's own, not ones written by the builders' model family
(D37).

**Model separation is not enforced by code yet.** D37 says config naming a builder
model for the holdout lens is refused, but `rusty_orch` has no holdout-lens kind or
that check today. A one-shot generator script bypasses it. D64 meets the rule in
substance for stage 1. A follow-up card should build the orch holdout-lens kind
with the check before a second task needs holdouts.

## 5. Suite shape

This section is the coverage check Nano reviews. The holdout model gets only the
profile names and their intent lines, never this table's wording (section 4).

Coarse profiles keep the one-bit-per-run signal small (#597 §2). Each profile is
one Python 3 script using only the standard library; `python3` comes from `/usr`,
already in `read_roots`. A script builds its fixtures at run time in a scratch
directory from a fixed seed, calls only `seam/run`, and checks JSON output and the
file system. A script prints one line per check and exits non-zero on any failure.

| Profile | Intent lines it covers |
|---|---|
| `holdout:match` | byte-identical groups (example 1: three identical files in three folders, one group, a suggested keeper); same name and size, different bytes is not a duplicate (example 2); the same files under two scan roots form one group across both (example 4, with two Linux roots standing in for D: and a share); links pointing outside the scanned roots are not followed (never 4) |
| `holdout:image` | same pixels with different or missing metadata is one `image` group (always 3); the group says which copy has richer metadata, and the keeper is it (always 4, example 3: `beach.jpg` with EXIF and `copy (2).jpg` without) |
| `holdout:safety` | nothing moves before a confirm (always 1); confirmed copies go to quarantine (always 2); confirming every copy of a group is refused, so the last copy survives (never 1); every original file's bytes are unchanged after scan and quarantine (never 2); purge acts only on quarantine (never 3) |
| `holdout:resume` | a scan killed with SIGKILL mid-way resumes on relaunch and ends with the same groups as a clean scan (target `crash_loss`); a rescan after adding files reports them (target `incremental_rescan`, behaviour only) |

Not here:
- **Performance** (first scan per TB, incremental rescan time, 500 GB tree): the
  `perf` profile and #1319.
- **Windows-only lines** (mapped drives, UNC paths, GUI): `golden:` cases or the
  Windows shadow run (D56, D63, D178). The open-time `untested` check (#1405) lists
  any left unmapped.
- **GUI and web UI responsiveness**: shadow run and Nano's judgement (D63).

Fixture sizes stay small (largest file 8 MiB in place of the 4 GB video), so a run
fits the profile's limits. Limits: `wall_secs` 900 per profile including the seam's
own build, other limits as `ProfileSet::shell` with `memory_bytes` 4 GiB.

## 6. Leak review

The suite must not tell builders how to build, and must not be tuned to one design.
Per D228:

- The reviewer is a local non-builder model that did not write the suite (Qwen
  when gpt-oss writes). It flags hints by category only (for example "names a data
  structure", "fixes an algorithm", "tied to one design's output layout beyond the
  contract").
- The author regenerates. Nobody edits test bodies.
- No Claude session reads the test bodies. This thread and Claude Code on BaileyAi
  handle the files by path and hash and see only category counts.
- Rounds are logged on the host only, never on the card, in the repo or in
  remind-me.
- Nano spot-checks the suite once (#596 V2). That is the human check.

## 7. Freezing and protected paths

Per D229:

- Stage 1 has no task yet, so the suite freezes at the real #1320 `bbp open
  --holdout-profiles --holdout-root`. `bbp holdout` only adopts a new version on
  an open task (PR #1419), so the card's "freeze it with `bbp holdout`" is drift.
- The suite is staged at `/srv/lel/dedup-manager-holdout/`, owned by orch-runner,
  outside every build-run mount. #1238's golden dir goes inside it
  (`golden/dedup-manager/`), because `holdout.rs` refuses a golden set outside the
  holdout root; #1238 chooses whether the Factory dir is copied in by digest.
- A dry open in a scratch BBP dir, with the same absolute root (the root path is
  part of the digest), proves the set is valid and gives the digest. It is recorded
  on #600 and in this file.
- **Protected globs** (600 design §1): `tests/frozen/**` (the visible frozen suite)
  and `golden/**`. `seam/` is not protected: it is the builders' code. Protected
  paths need a pinned base, so the open uses `--base` with the intent commit.
- In the same scratch task, a dry candidate that adds `tests/frozen/x` must be
  refused `protected_path` with the path named. That is the card's done check.

The same frozen suite serves both arms of #596 (D57: "holdout no worse on 5 of
5"), and it is the operator-supplied holdout input for D44 cleanups while Q37 is
open (D113).

## 8. Slices

| Slice | What | Where | Gate |
|---|---|---|---|
| 0 | This design (docs PR) | cloud | Nano approves on the card |
| 1 | Intent v0.2-stage1 text (per Q1, Q2), the generator prompt and script, staged in `docs/projects/let-em-loose/1236/`; the new `dedup-manager` commit | cloud, then BaileyAi | Nano approves the intent text and runs the commit prompt (outward step on his host) |
| 2 | Generate with the holdout model, leak review rounds, Nano's one spot-check | BaileyAi (local model) | Nano spot-checks |
| 3 | Stage the suite, dry open with protected globs, dry candidate refused, digest recorded on #600 and here | BaileyAi | none beyond D131 |

No Rust code changes. Slice 1 and 3 hand Nano paste-ready prompts in
`/mnt/project-files/let-em-loose/1236/`; anything he types is bash (BaileyAi runs
Fedora).

## 9. For Nano

Three decisions, posted as decision cards in the project thread:

1. **Test seam (Q1).** a) separate `SEAM.md` second commit; **b) intent v0.2-stage1
   adds a scriptable CLI line and the contract (recommended)**; c) web UI HTTP API.
2. **Toolchain (Q2).** Decided: Rust stable, pinned (D244).
3. **Holdout model (Q3).** Decided: gpt-oss-120b, Qwen fallback, local (D245).

Still open elsewhere and not decided here: `taste: pending` in the intent, Q4
(golden sign-off, #1238) and Q37 (cleanup holdouts).

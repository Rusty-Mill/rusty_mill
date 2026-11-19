# #600 build plan: builder sessions for the dedup experiment

Status: build plan, 2026-11-19 (card #1402). The approved design
(`600-builder-session-design.md`, approved 2026-11-04 as D66) lived only in
`/mnt/project-files/` and is gone (OPEN_QUESTIONS Q44). This plan rebuilds it from
what survives: DECISIONS D56, D63–D66, D101, D104, D107, D108, D160–D162,
`mux-hosted-agents-design.md` (§2, §3, §4, §6, §11), `1571-launch-profiles-design.md`,
the claims addendum (C-R4, **Runs and candidates**) and card #600's comments.
Where it fills a gap, it says so and marks it **[filled]**. D66's approval covers
the §8 list named in section 7; nothing here adds to that list. Every **[filled]**
item goes to Nano before the card that needs it is built (section 8).

Drift noted: D164 (the freeze lift that releases #1402) is copied onto this
branch from PR #1813, which holds D163 and D164 and isn't merged yet. Expect a
trivial conflict in D164 when it merges (Nano concurred, 2026-11-19).

## Ownership

| Piece | Owner | Why |
|---|---|---|
| `Builder` job kind, admission, config values | orch (`orch-lens`, `orch-core`) | Jobs and the schedule are orch's. |
| `builder_session` table | orch (`orch-store`) | orch's state, behind the store port. |
| Builder launch, `jail.json` writer | orch-bbp, with `orch-jail` (D107) | One `JailSpec` type for writer and reader. |
| Slot, cgroup holder, `quiesce` | BBP host (`rusty_bbp_host`) | D108; unchanged by this plan. |
| Pty hosting | rusty_mux | orch decides, the mux hosts (D104). |
| Open-time `untested` check | BBP host | Reads the intent when a claims task opens. |
| Experiment repo, holdouts, golden set, test tree | experiment harness | Outside builders' reach. |

## 1. What a builder session is

A Claude CLI session (D64: Claude only in v0; Codex doesn't build) that builds one
candidate for one claim in a per-candidate worktree. It runs jailed, as the
process of a mux-hosted pty (D104). The stored candidate names its claim, so the
runner's `test_report` on it becomes `executed` evidence (C-R4). A builder's own
claim that its tests pass stays opinion. Each run is debited from the claim's
budget (card #600 scope).

Builders see the intent repo only: no `rusty_fclone`, no `rusty_mill` (#1234).
They see a summary of the holdout suite, never the suite (#1236).

## 2. The `Builder` job kind and admission (#1404)

A new `Slot` kind in orch's schedule, next to the lens jobs (`orch-lens`
`schedule.rs`). A job is one builder session for one claim. Domain code stays
free of I/O; admission is a pure function over counts and config.

Admission refuses a new session when any of these holds:

- the claim already has `builders_per_claim` sessions, live or stored;
- `max_live_builders` sessions are live across all claims;
- the claim's budget can't cover a session (**[filled]**: the refusal reuses the
  existing budget refusal; no new budget rule).

Tests: each limit at, one under and one over its boundary; the refusal names which
limit.

## 3. Config values (#1404)

Names approved in D66. Defaults:

| Value | Default | Source |
|---|---|---|
| `builder_wall_secs` | 2700 (45 min) | D66 §10 default, `[default]`. |
| `builders_per_claim` | 2 **[filled]** | Competing prototypes need at least two (card #600 scope). 1124's spend estimate notes the value isn't in the repo. |
| `max_live_builders` | 2 **[filled]** | One claim's candidates at once; matches the agent slot count the design assumes. Nano sets the real number. |

The wall clock is enforced on two separate paths (D104 §3, D108 item 1):

- **Normal path, orch:** SIGTERM through `SignalBlock`, a 60 s grace, then
  `cgroup.kill`.
- **Crash path, the holder:** no grace and no signal. From the deadline it writes
  `cgroup.kill` at every look that doesn't find the cgroup empty, and retries until
  it is empty, then frees the slot.

## 4. `builder_session` table (#1404)

orch-store table, one row per session. Name approved in D66. Columns are
**[filled]** (the design's column list is lost): `claim`, `n` (session number
within the claim), `candidate` (set when stored), `state` (`admitted`, `running`,
`stored`, `failed`, `aborted`), `started`, `ended`, `cost` (from ACP usage on fd 4,
D104 §3 "Reap and pricing"), `human_touched` (D104 item 2, derived, not stored
twice: **[filled]** read it from the claim fold instead of a column if D106 allows).

Session files live under `<dir>/builders/<claim>/<n>/`: `jail.json`, `acp.log`,
the witness digests (Q13, resolved by D104). A mux restart aborts the turn,
debits nothing and retries the claim once (mux-hosted-agents-design §3).

Tests: round-trip, resume after a crash mid-session, the abort-and-retry-once rule.

## 5. Windows-only intent lines (#1405)

D56 and D63: behaviour only a Windows run can judge (mapped drives, UNC paths,
the desktop GUI) is covered by a read-only shadow run on Nano's Windows machine
and golden cases he signs off (#1321, #1238). Linux claims runs judge the engine
and the web UI.

At claims open, the BBP host checks each intent line. A line judgeable only on
Windows is marked `untested` for the Linux runs; a Windows-specific line with
neither a shadow-run nor a golden case counts as untested. **[filled]**: how a
line is recognised as Windows-only (a marker in `INTENT.md` versus a check on its
text). Nano's frozen intent can't change (#600: any change is a new version), so
the likely answer is a host-side list keyed by line, not a marker in the intent.

D65: a Windows result that contradicts a Linux pass reaches the claim by Nano's
human `decision reopen` (C-R8), after `budget_extended` only if the budget is
exhausted. It is never posted as `executed` evidence, as it has no runner report
bound to the candidate (C-R4).

Gate: none beyond D66. A new event field is a BBP schema change and ask-first; if
the check needs one, stop and escalate.

## 6. Builder launch (#1318)

Follows D104 and mux-hosted-agents-design §2; nothing new is decided here.

1. **Admit** (section 2).
2. **Prepare.** Worktree, `cgroup-open` (D108), orch-minted token, `jail.json`
   written with `JailSpec` (D107).
3. **Hold.** Start the cgroup holder (`bbp slot-hold --cgroup`). The holder takes
   the agent slot itself through the existing `slots.rs`; orch duplicates that open
   file description with `pidfd_getfd` and passes it to the mux as fd 3 (D108
   item 2, whose amendment assigns this hand-off to this slice). A refused
   duplication (for example a Yama `ptrace_scope` of 2 or more) fails the launch
   closed: never launch without fd 3. This replaces the design §2 order, where orch
   took the slot first.
4. **Open.** One `Launch` through a fresh `rusty-mux-ctl own` child (#1571 §2):
   program `lel-jail <jail.json>`, exact env with no `RUSTY_MUX_*` (D13, ADR-0022),
   fd 3 slot, fd 4 ACP socketpair, `hold: true`, `input: owner-only`.
5. **Run.** ACP over fd 4, rendered to `acp.log` (a display aid, not a record).
   The witness (upper-layer diff, exec log, gateway log, D101) is the #726 target
   and is deferred with those layers (D104 item 7, section 7); the v0 slice
   doesn't record it.
6. **End.** SIGTERM via `SignalBlock`, 60 s grace, `cgroup.kill`, `Release` (never a
   mux kill), holder frees the slot.

Profile: the `claude-builder` agent file and project file in Factory
(D160–D162, `same_user` v0, D161). Under `jail` the project's `credentials` are
ignored (D101); the agent gets only the orch-minted token.

**Dependency mirror.** The jail has no route to a registry in v0, but
`InternetOnly` still allows egress (D107 known gap). The pre-fetched mirror is
approved (D66); its contents are **[filled]**: the dependencies INTENT.md's
toolchain needs, fetched before launch and mounted read-only.

Waits on: mux rows C75 #749, C76 #750, C78 #752. Q15 is answered (D165): the
Claude CLI uses Nano's subscription. Delivery is settled for v0 (D166): the CLI
reads Nano's `~/.claude` login through a read-only root in `jail.json`, a written
exception to D101 scoped to `same_user` v0 that expires when the gateway (#726)
lands; `jail.json`'s `token` slot still carries only an orch-minted token (D107).
Risk accepted: v0 is not a security boundary and egress is open (the D107 gap), so
a jailed builder could read and send the login. One paid run is approved (D167):
the #1318 toy-claim run (2 builders, 45 min, one claim), once Nano adds the Claude
`no_overage` assertion on BaileyAi (open action, not done). #1320 is not approved.

Tests (design §11): inside the jail `RUSTY_MUX_SOCKET` is unset, its path doesn't
exist (needs the mount namespace; Landlock covers v0, D107), `socket(AF_UNIX)`
fails; a mux-restart abort; slot release on an empty cgroup; the deadline holding
with orch killed; a builder for a toy claim runs under `lel-jail` and its
`acp.log` renders.

## 7. D66's approved list, and what builds it

| D66 §8 item | Card |
|---|---|
| orch `Builder` job kind and admission check | #1404 |
| `builders_per_claim`, `max_live_builders`, `builder_wall_secs` | #1404 |
| orch-store `builder_session` table | #1404 |
| Open-time `untested` check for Windows intent lines | #1405 |
| orch-bbp builder launch profile with a dependency mirror | #1318 |
| Local-only repo `dedup-manager` on BaileyAi | #1234 |

The §10 defaults (local-only repo, 45-minute wall clock, mirror) stay `[default]`.
Not on D66's list, so not built here: lens sessions in blocks (D104 item 8), the
#726 layers (overlay, gateway-only egress, witness), Codex as builder (D64).

## 8. Experiment setup (the stage-1 inputs)

| Card | What | Gate |
|---|---|---|
| #1234 | Fresh repo holding only `INTENT.md` = frozen v0.1-stage1, byte for byte; the repo is `dedup-manager`, local-only on BaileyAi (D66 §10). | Creating a repo is outward: Nano creates or OKs it first. |
| #1236 | Holdout suite written by a holdout lens on a model no builder uses (D37); frozen with `bbp holdout`; protected test globs. | Paid calls need Nano (D134). |
| #1238 | Golden set in the MANIFEST/SIGNOFF format `bbp golden-run` checks (#1421); closes Q4. | Nano signs off. |
| #1319 | Seeded generator for a 500 GB tree shaped like Nano's drive, the named reference hardware and the measurement protocol. | Aggregate stats from Nano's drive only, no file names unless he agrees. |

The frozen targets they measure: first full scan at most 30 min/TB; incremental
rescan under 1 h after adding 50 GB; crash loss at most a few minutes.

## 9. Slicing and order

```
#1234 repo + INTENT.md ──┬─► #1236 holdouts
                         ├─► #1238 golden set (Q4)
                         └─► #1319 synthetic tree
#1402 this plan ─► #1404 Builder job kind ─┐
              └──► #1405 untested check    ├─► #1318 builder launch via mux ─► #1320 the run
                                           │       (waits on #749, #750, #752 and Nano's no_overage assertion, D167; #1320 also waits on M1 and the #1318 result)
#1236, #1238, #1319 ───────────────────────┘
```

#1404 and #1405 start first: both are approved (D66), need no host or mux work
and build in the cloud. #1318 builds on #1404. #1320 (the run) also waits on M1
(#1166, #1124). #1321 (Windows shadow run) and #1322 (report) follow #1320.
One small PR per slice with a change fragment, review requested from
dottiusmaximus; agents never queue or merge.

## 10. For Nano before build

Everything marked **[filled]**: the two builder counts (section 3), the
`builder_session` columns (section 4), how Windows-only lines are recognised
(section 5), the mirror's contents (section 6). Open and unchanged: Nano's Claude `no_overage`
assertion on BaileyAi (D167), Q4 (golden-set sign-off), and the model for the
holdout lens. Settled 2026-11-19: Q15 delivery (D166, v0 only) and the #1318 paid
run (D167).

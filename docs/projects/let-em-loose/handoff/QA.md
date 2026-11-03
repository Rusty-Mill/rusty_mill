# Implementer Q&A: let-em-loose

Build-time questions not already answered in [../QUESTIONS.md](../QUESTIONS.md) (Q1–Q31). Each answer cites its source. `mem_…` ids are remind-me memories; "card #N" means the Mill Board card and its comments.

## Erosion gate (#704)

- **Q:** What blocks a claim on erosion in v0? **A:** Only a failing `erosion` profile, through the existing fail-block: a candidate whose erosion delta against the pinned baseline exceeds a limit fails that profile, and the claim can't be accepted until a later run passes it. Until the limits are calibrated the profile is report-only and always passes, so nothing blocks. Code in an unsupported language is `unmeasured` and never fails or escalates. There is no in-band override; a blocked claim needs a better candidate or reaches the inbox through budget/stuck, where Nano's Approve (a human `decision accept` or `choose`, which bypasses the block) is the override. A claims task also can't open without an erosion profile or a recorded `erosion = "off"` opt-out. [agreed] D41, D42, D43, D45, D30; claims addendum § Failing runs block, C-R8. What happens when report-only reaches its time box is open (OPEN_QUESTIONS Q10).
- **Q:** What does it measure? **A:** Two numbers in basis points: CC concentration (share of cyclomatic complexity in functions above CC 10) and duplication (share of tokens in repeated normalized 50-token windows), as deltas against the baseline. It also reports total CC, LOC and the worst functions. [agreed] D41; card #704 Build 1
- **Q:** Is there an override or an `erosion` claim domain? **A:** No, not in v0. An `erosion` domain like `tests` is the v1 option if overrides become common. [agreed] D43
- **Q:** What limits before calibration? **A:** None enforced: report-only for 2 weeks or 10 tasks, whichever comes first, then calibrate from rusty_mill's merged human PRs (proposal: p90 delta) and Nano sets the limits. The fixed +150/+100 bp values were rejected. [agreed] D42; card #704 Build 5; erosion-gate-design §6, §8
- **Q:** Does it change the BBP schema? **A:** No. The thresholds are profile args covered by `profile_digest` and frozen at task open. [agreed] D41; erosion-gate-design §3, §9
- **Q:** Which languages and files? **A:** Rust only. Non-code files are ignored. `#[cfg(test)]` modules and `tests/` are skipped. Other languages are added when a real task needs one. [agreed] D45; card #704 Build 1
- **Q:** Python for the SlopCodeBench spike? **A:** Add Python support to `rusty_erosion` for #705; don't port the problems to Rust. [default] card #705 comment (agreed in voice 2026-11-03, Nano can overrule)
- **Q:** New dependencies? **A:** None: `tree-sitter` and `tree-sitter-rust` are already workspace dependencies (`Cargo.toml`). The new crate itself needs approval as listed in the design. [agreed] card #704 Build 1; erosion-gate-design §9
- **Q:** When does cleanup run? **A:** orch writes a `consolidate` intent at half the drift limit or after 5 accepted claims, whichever comes first. The frozen suite and holdouts must stay green, and limits are negative so erosion must go down. [agreed] D44
- **Q:** What does an erosion escalation show? **A:** The per-metric delta and the three worst-affected functions, plus `unmeasured` code on the claim. [agreed] D43; card #704 Build 3
- **Q:** What tests are required? **A:** Fixtures, a renamed clone still detected, the 49 vs 50 token boundary, determinism, and a real-host test where a pasted duplicate is reported and later blocked once enforcing. [agreed] card #704 Done when

## Lens runner (#594)

- **Q:** Can a lens call a paid API? **A:** No. v0 is flat-rate subscriptions and local models only; config refuses any nonzero price, and CLI lens calls need a verified subscription login. No paid calls until Nano says so. [stated] Nano 2026-11-03 10:54; mem_cd2ec8a25fcd4c15971a8cd39e6952cc; mem_d6473d0f027344d09581b4b0027a4e9a
- **Q:** Where does Codex run? **A:** Only inside the rootless podman container `lel-codex` on BaileyAi, by `podman exec` on the checked container id. rusty_orch runs host-side as `orch-runner`. The runner checks mounts, machine-id and privilege before every call and fails closed. [agreed] mem_e36ba8d9bb10404e93c5f0991259c75a (Nano 2026-11-03 12:54); `594-codex-container-design.md`
- **Q:** Who sets the operator assertions (`no_overage`, `host_cleared`)? **A:** Only Nano; both are off by default and agents can't set them. `no_overage` expires after 30 days. [stated] Nano 2026-11-03 10:54; mem_01e7934bf36d4daebc236c49b8e324a2
- **Q:** What's next on #594? **A:** The `rusty_orch lens` subcommand, then persisted runner state, the observe-only breaker, lessons, and the `task_paused` host write (#598 schema items need approval first). [agreed] card #594 (2026-11-03 13:00); mem_c42b8e1059c141c1b521b708670f43cc
- **Q:** Which lenses propose? **A:** Only the Architect: `proposers: [claude, local]` plus a `rebel` from config, which can't repeat a proposer. [agreed] D34; card #594 revision 3

## Inbox (#595)

- **Q:** How is the inbox sorted across tasks? **A:** Per task: irreversible first, then tasks in the order given, oldest first within a task. Ages aren't compared across tasks; that schema change waits until more than one claims task escalates at once. [stated] card #595 (Nano 2026-11-02 20:22Z)
- **Q:** Remediate diffstat and BBP decision ids? **A:** Diffstat is deferred. Decision ids for settling an uncertain send are card #703, backlog, not blocking v0. [agreed] card #595 (2026-11-03)

## Gaps (#598, #599)

- **Q:** Where do the BBP host, claims runs, shadow run and daily watcher run? **A:** The agentos container on the MS-01 (LXC 107); BaileyAi bare metal if the sandbox probe fails there. Not deployed. [stated] cards #598, #599 (Nano 2026-11-02 22:08Z)
- **Q:** What happens if the baseline suite is red at open? **A:** The open is refused, listing the red tests and the one profile-set edit that moves them out. [stated] card #598 Q1
- **Q:** Can a cloud lens see real data? **A:** No. Cloud providers are cleared to `internal`; real data, including names, paths and shadow plans, is `sensitive` and stays local. [stated] card #598 Q6
- **Q:** Does the cost breaker enforce in the first #600 run? **A:** No, it's observe-only and logs would-trips. The meter and the $200/$25 ceilings enforce from the first paid call. [stated] card #598 Q4
- **Q:** Are lessons reviewed before they're written? **A:** No. They're written automatically; Nano deletes unwanted ones. [stated] card #599 Q1

## Dedup experiment (#600)

- **Q:** Has the intent interview happened, and what are the targets? (supersedes ../QUESTIONS.md Q26, Q27) **A:** Yes, 2026-11-02. Intent `dedup-manager` v0.1-stage1 is frozen: exact duplicates plus same-image-different-metadata, local drives and NAS, Windows first, first scan at most 30 min/TB, incremental rescan under 1 hour after 50 GB, no taste domains. It is committed unchanged as INTENT.md when the experiment repo is created; any change is a new intent version. [stated] card #600 (Nano 2026-11-02)

## remind-me (#592)

- **Q:** Has the write to existing records happened? (supersedes ../QUESTIONS.md Q31) **A:** Yes. `project = let-em-loose` was written to exactly the 52 listed memories on 2026-11-02 21:49Z, 0 failures. [agreed] mem_ff0f72935a2048678c9676a00edcf69e; card #592

## Process

- **Q:** Where do new decisions go? **A:** As D46+ in `docs/projects/let-em-loose/DECISIONS.md`, written only by the `let-em-loose-expert` subagent. Never create `docs/DECISIONS.md`. [default] D49, D50, D51
- **Q:** How are PRs shaped? **A:** One small PR per slice, stacked on the previous slice's branch, naming the PR it sits on, retargeted to main when that merges; each builds and tests on its own. Update a stack by merging the lower branch up, never by rebasing. [stated] Nano 2026-11-03 (remind-me); AGENTS.md
- **Q:** Which trace numbers are free in the claims addendum? **A:** Grep `crates/libs/protocol/rusty_bbp/docs/spec/bbp-claims-addendum.md` on main for the highest `CL-` and use the next one (CL-101 on 2026-11-03). Ranges were split once after a collision: #597 CL-71..80 and CL-90+, inbox CL-81..89. [default] mem_57dd2f10240f4f9988c3a10042148210

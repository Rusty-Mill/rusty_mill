# Let Em Loose: decisions

Each entry gives the decision, why it was made, and what was rejected. **Agreed** records the original author's report of Nano's approval; it is not independent verification or an operational authorization. **Proposed** means it is the working direction but still awaits a design or confirmation. From D46 on, entries carry a project-handoff tag instead ([handoff/DECISIONS.md](handoff/DECISIONS.md)): `[stated]` or `[agreed]` (Nano's, with an attributable reference; ask before changing) or `[default]` (an agent's choice; revise with a new entry saying why), plus the remind-me id and a key. Only the `let-em-loose-expert` subagent appends entries; recording is best effort and duplicates are reconciled by key. Dates are when the decision was made. Where an earlier entry and a later one conflict, the later one wins. The detailed BBP rules are in the claims addendum, `crates/libs/protocol/rusty_bbp/docs/spec/bbp-claims-addendum.md` (PR #1386).

## Authority, provenance and current contracts

The original author reported the design approvals below in [#1384](https://github.com/Rusty-Mill/rusty_mill/pull/1384). The external [source concept](https://claude.ai/artifact/JoCPd4weXwkJ7Zy8qMjiJJ) and Mill Board card numbers are provenance pointers, not independently verified approval records in this bundle. Where no attributable approval is linked, the reported approval remains subject to owner verification. In particular, D41–D45 originated as the author's report of #704 approval. The later D42 expiry and D44 zero-floor choices are explicitly recorded by the coordinator in [the owner-decision record](https://github.com/Rusty-Mill/rusty_mill/pull/1384#issuecomment-6614620497), including its source/confirmation reference.

Repository rules in [AGENTS.md](../../../AGENTS.md) and [CONTRIBUTING.md](../../../CONTRIBUTING.md) govern contributions. Merged BBP and orch contracts govern their implemented behavior; this concept bundle and the mutable external doc cannot supersede them. Later decisions supersede earlier concept text only within that authority boundary. This documentation correction authorizes no implementation starts, live provider calls, spending, credentials, deployment or settings changes. Separately approved implementation work retains its own authorization and review gates.

Precise contracts used by this guide:

- [BBP claims addendum](../../../crates/libs/protocol/rusty_bbp/docs/spec/bbp-claims-addendum.md): C-R4 and **Runs and candidates** for authenticated executable evidence; **Reads, delta and subscriptions** for tool-read visibility; **Failing runs block**, C-R8 and **Action marks** for human overrides and fenced state changes; **Budget** for claim allowances. These are merged contracts, rather than new permission from this guide.
- [Provider IDs, ADR-0019](../../../crates/apps/rusty_orch/docs/adr/0019-provider-ids.md): save-time conversion and rollback limits.
- [Lens core, ADR-0021](../../../crates/apps/rusty_orch/docs/adr/0021-lens-runner-core.md) and [operator assertions](../../../crates/apps/rusty_orch/crates/rusty_orch/src/lens/assertions.rs): zero-rate v0 admission and operator-controlled no-overage/host-clearance checks.
- [Inbox action planning](../../../crates/apps/rusty_mill_board/src/inbox.rs) (`plan`, `plan_task`, `Total::extend`) and [BBP CLI adapter](../../../crates/apps/rusty_mill_board/src/bbp_cli.rs): revision-fenced previews/actions, per-reason behavior and whole-task totals.

| # | Decision | Status |
| --- | --- | --- |
| D1 | Lenses, not stages | Agreed 2026-11-01 |
| D2 | Converge on claims, not conversation | Agreed 2026-11-01 |
| D3 | The board computes confidence | Agreed 2026-11-01 |
| D4 | Budget is the price of certainty | Agreed 2026-11-01 |
| D5 | Nano writes intent and judges exceptions | Agreed 2026-11-01 |
| D6 | Keep dissent honest | Agreed 2026-11-01 |
| D7 | Several blind candidates, including a rebel | Agreed 2026-11-02 |
| D8 | The board is the context | Agreed 2026-11-02 |
| D9 | Planning, building and testing are concurrent | Agreed 2026-11-02 |
| D10 | Research: learn, don't inherit | Agreed 2026-11-01 |
| D11 | BBP is a pure protocol | Agreed 2026-10-11, reaffirmed 2026-11-02 |
| D12 | orch is model-independent, via ports and adapters | Agreed 2026-11-02; providers refined in D34 |
| D13 | No agent-to-agent calls | Agreed 2026-11-02 |
| D14 | Tool-using agents join through the board over MCP | Proposed 2026-11-02; decided 2026-11-04, after v0 (D60) |
| D15 | Concurrent, event-driven evidence posting | Agreed 2026-11-02 |
| D16 | Value-driven scheduling is a shadow experiment | Agreed 2026-11-02 |
| D17 | Build in the rusty_mill monorepo | Agreed 2026-11-02 |
| D18 | The dedup experiment gets a fresh repo and intent only | Agreed 2026-11-01 |
| D19 | The Factory repo holds process and config only | Agreed 2026-11-02 |
| D20 | No fourth executor: extend orch (reopened by D103, re-closed by D104) | Agreed 2026-11-02; reopened 2026-11-04 (D103); re-closed 2026-11-05 (D104) |
| D21 | Thin slice first, then let it build itself | Agreed 2026-11-01 |
| D22 | Ground truth sits outside the agents' loop | Agreed 2026-11-02; designed in D37 |
| D23 | Defer existing code and the smaller gaps | Agreed 2026-11-02 |
| D24 | Audit remind-me before building | Agreed 2026-11-01 |
| D25 | Design first on every card | Agreed 2026-11-02 |
| D26 | The name is Let Em Loose | Agreed 2026-11-02 |
| D27 | Claims sit beside the stage lifecycle | Agreed 2026-11-02 (#593) |
| D28 | Replay determinism, not order independence | Agreed 2026-11-02 (#593) |
| D29 | Scoring and budget defaults | Agreed 2026-11-02 (#593) |
| D30 | A failing test blocks acceptance | Agreed 2026-11-02 (#593) |
| D31 | Superseding a claim | Agreed 2026-11-02 (#593) |
| D32 | When a dependency collapses | Agreed 2026-11-02 (#593) |
| D33 | Action marks and held reruns | Agreed 2026-11-02 (#593) |
| D34 | Lenses, providers and local models | Agreed 2026-11-02 (#594) |
| D35 | The inbox is a read view over BBP in Mill Board | Agreed 2026-11-02 (#595) |
| D36 | Value scheduling lives in orch, with a pre-registered rule | Agreed 2026-11-02 (#596) |
| D37 | Holdouts, protected tests, golden sets, calibration | Agreed 2026-11-02 (#597) |
| D38 | Phase B intents and spend | Agreed 2026-11-02 (#596) |
| D39 | remind-me gets an explicit project parameter | Agreed 2026-11-02 (#592) |
| D40 | Build the slice before designing more | Agreed 2026-11-02 |
| D41 | Gate on code erosion, not only behaviour | Agreed 2026-11-03 (#704) |
| D42 | Erosion limits come from our own history | Agreed 2026-11-03 (#704) |
| D43 | No erosion-specific override; budget approval is separate | Agreed 2026-11-03 (#704) |
| D44 | Scheduled cleanups pay erosion down | Agreed 2026-11-03 (#704) |
| D45 | Rust only; unmeasured code is reported, not failed | Agreed 2026-11-03 (#704) |
| D46 | Three-tier escalation for design questions | [stated] 2026-11-03 (handoff) |
| D47 | Ask the project expert before Nano | [stated] 2026-11-03 (handoff) |
| D48 | Never invent a decision; unsourced goes to OPEN_QUESTIONS | [stated] 2026-11-03 (handoff) |
| D49 | The handoff pack is scoped to the project folder | [default] 2026-11-03 (handoff) |
| D50 | One decision log, D-numbering continues | [default] 2026-11-03 (handoff) |
| D51 | The expert is the single writer of settled answers | [default] 2026-11-03 (handoff) |
| D52 | rusty_erosion location and core/adapter module split | [default] 2026-11-03 (#704) |
| D53 | One host: everything deploys on BaileyAi | [stated] 2026-11-03 (#598, #599, #594) |
| D54 | Lens shell reaches BBP through one `bbp mcp` session per lens | [default] 2026-11-04 (#594) |
| D55 | Orch-originated intents are parked | [stated] 2026-11-03 (general) |
| D56 | Windows-specific #600 behaviour is tested by a Windows shadow run and golden cases | [stated] 2026-11-03 (#600) |
| D57 | #596 adds an erosion pass rule: no worse than baseline on 4 of 5 runs | [stated] 2026-11-04 (#596) |
| D58 | #599 §6 schema and API changes approved | [stated] 2026-11-04 (#599) |
| D59 | #598 §6: task_paused, intent_revised and data classes approved | [stated] 2026-11-04 (#598) |
| D60 | Tool-using agents join over MCP after v0 ships | [stated] 2026-11-04 (general) |
| D61 | #708 alien review lens parked until v0 runs (superseded by D74) | [stated] 2026-11-04 (#708) |
| D62 | The rest of #598 §6 approved | [stated] 2026-11-04 (#598) |
| D63 | #600's Windows GUI is judged only on Windows | [stated] 2026-11-04 (#600) |
| D64 | #600 builders are Claude CLI only in v0 | [stated] 2026-11-04 (#600) |
| D65 | A Windows result that contradicts a Linux pass reaches the claim by Nano's reopen | [stated] 2026-11-04 (#600) |
| D66 | The #600 builder-session design is approved (launch path reopened by D103, re-closed by D104) | [stated] 2026-11-04 (#600); reopened 2026-11-04 (D103); re-closed 2026-11-05 (D104) |
| D67 | #722 drops the claim/evidence hash DAG; a linear per-line chain only | [stated] 2026-11-04 (#722) |
| D68 | #722: a broken chain fails every load | [stated] 2026-11-04 (#722) |
| D69 | #722: chained line framing, no BBP schema bump | [stated] 2026-11-04 (#722) |
| D70 | #722 adds `bbp verify` | [stated] 2026-11-04 (#722) |
| D74 | #708 alien review lens unparked (supersedes D61) | [stated] 2026-11-04 (#708) |
| D75 | #708 alien review lens design v1 approved (shadow-only) | [stated] 2026-11-04 (#708) |
| D76 | `lens run` merges as a disabled slice until the accept-line source lands | [stated] 2026-11-04 (#594) |
| D77 | `claim_card` carries each claim's accept line and each evidence item's note and strength | [stated] 2026-11-04 (#594) |
| D78 | #708 alien hunt runs as a runner pass, outside the scheduler | [default] 2026-11-04 (#708) |
| D79 | #599 watcher posts as a `watcher` roster role | [default] 2026-11-04 (#599) |
| D80 | #599 maintenance tasks are opened by the human and named in `factory/apps/<app>.json` | [default] 2026-11-04 (#599) |
| D81 | #599 lesson trigger 2 detects reverts from the action mark and git | [default] 2026-11-04 (#599) |
| D82 | #704 slice 2: erosion profile recognition, invocation, opt-out, base, measured set and modes | [default] 2026-11-04 (#704) |
| D83 | #599 watcher dedups by finding key and slot in `watch_finding` | [default] 2026-11-04 (#599) |
| D86 | #598 build defaults while 598-gaps-design.md is unavailable | [default] 2026-11-04 (#598) |
| D88 | #704 D42 expiry pause: go-live, count, what pauses, opt-out | [default] 2026-11-05 (#704) |
| D91 | #704 slice 3: erosion escalation detail in the inbox | [default] 2026-11-05 (#704) |
| D92 | #599 track records: job rows, pivotal scope, ground truth, weight, PR split | [default] 2026-11-05 (#599) |
| D93 | #599 track records score pivotal reviews against ground truth only | [stated] 2026-11-04 (#599) |
| D94 | Let Em Loose designs live in the repo, in their first draft PR | [stated] 2026-11-04 (general) |
| D96 | #708 orch runs the alien lens's probes itself, sandboxed | [stated] 2026-11-05 (#708) |
| D97 | #708 slice 4d: probe scratch trees and cargo time limit | [default] 2026-11-05 (#708) |
| D98 | #708 alien hunt reads the candidate and its diffs through `get_artifact` | [default] 2026-11-05 (#708) |
| D99 | #708 slice 4c: `lens run --alien`, its config entry and its trigger | [default] 2026-11-05 (#708) |
| D100 | Design mux-hosted agent ptys (D20 and D66's launch path reopened for that design) (superseded by D103) | [stated] 2026-11-04 (mux, #600) |
| D101 | Tools go through the gateway, and the agent process is jailed | [stated] 2026-11-04 (#726) |
| D102 | orch and the gateway stay separate | [stated] 2026-11-04 (#726) |
| D103 | Agents get mux-hosted ptys (supersedes D100; reopens D20 and D66) | [stated] 2026-11-04 (mux, #600, #726) |
| D104 | The mux-hosted agent ptys design is approved (re-closes D20 and D66) | [stated] 2026-11-05 (mux, #600, #726, #732) |
| D105 | Takeover events go under claims log schema 5, with no schema 6 bump | [default] 2026-11-05 (mux, #732) |
| D106 | `human_touched` is derived in the fold, not stored | [default] 2026-11-05 (mux, #732) |
| D107 | `lel-jail` v0 launch contract: `orch-jail` crate, `jail.json` fields, setup order, tests | [default] 2026-11-05 (#600, #732) |
| D108 | #600 cgroup holder, slot hand-off and cgroup `quiesce` | [default] 2026-11-05 (#600, #732) |
| D109 | Takeover end drains delivered input through `takeover_closing` (amends D104 item 2) | [stated] 2026-11-05 (mux, #732, PR #1502) |
| D110 | #704 slices 4–5: cleanup trigger and calibration build defaults | [default] 2026-11-05 (#704) |
| D111 | #599 `rusty_orch lens record`: read-only calibration report command | [stated] 2026-11-05 (#599) |
| D112 | #599 `claim_history` gains `min_authors`, golden results, evidence refs, reopen wakes and candidate provenance | [stated] 2026-11-05 (#599) |
| D113 | #704 slice 4: cleanup mode, consolidate inputs, split | [default] 2026-11-05 (#704) |
| D114 | #704 D44 cleanup half-limit leg: repo head vs last cleanup | [stated] 2026-11-06 (#704) |
| D115 | #704 approved erosion limits: a human-only host record | [stated] 2026-11-06 (#704) |
| D116 | A GUI shows the LEL stage strip as a derived read-only display and the product intent as a read-only brief | [stated] 2026-11-04 (general) |
| D117 | #704 `rusty_orch consolidate`: public subcommand approved as built in PR #1539 | [stated] 2026-11-06 (#704) |
| D118 | #704 D115/D114 build: the limits record, its command, the open check and orch's half-limit leg | [default] 2026-11-06 (#704) |
| D119 | #704 `bbp erosion approve` and `bbp erosion limits`: both commands and their trust model approved as built in PR #1543 | [stated] 2026-11-06 (#704) |
| D120 | #704 slice 4d: half-limit leg firing rule, measuring, `--bbp-bin` and pending base | [default] 2026-11-06 (#704) |
| D121 | #704 slice 4d: `rusty_orch consolidate` half-limit leg changes approved as built in PR #1544 | [stated] 2026-11-06 (#704) |
| D122 | Mill Board `external_refs`: observation-only references for non-Orch execution, approved in principle | [stated] 2026-11-10 (general, #1432) |
| D123 | #1432 S5 design defaults: ADR location, providers, capability, summary flags, card_take | [default] 2026-11-11 (#1432) |
| D124 | #1432 S5 external refs design (ADR-0006) approved as written for implementation | [stated] 2026-11-11 (#1432) |
| D125 | #1432 S5 clarifications: `unknown` refs not evictable, S5a document readback, refs on the domain `Card` | [default] 2026-11-12 (#1432) |
| D126 | #1432 S5: card summaries drop `has_execution` and `has_external_refs` (supersedes D123 item 4) | [stated] 2026-11-12 (#1432) |
| D127 | Design freeze: no new Let Em Loose design until its first run | [stated] 2026-11-12 (general) |
| D128 | M1 committed: #758 items 1-3, #1123, #1124, #1166 to Ready; #1092 waits for the BaileyAi host; #754 out | [stated] 2026-11-12 (general, M1) |
| D129 | Lost designs not needed for M1; #1402 builder-session rebuild waits until after the first run | [stated] 2026-11-12 (general, M1) |
| D130 | M1 design steps approved as one batch of runbooks and config (changes D25 for M1) | [stated] 2026-11-12 (general, M1) |
| D131 | Deploy on BaileyAi authorised for M1 (protocol host, orch, Mill Board inbox); BaileyAi confirmed as host; #1092 released | [stated] 2026-11-13 (general, M1) |
| D132 | #1092: Mill Board runs as its own user (`mill-board`) on BaileyAi, reaching `bbp` through a narrow sudo rule | [stated] 2026-11-16 (1092, M1) |
| D133 | Lens, domain and skill packs load lazily; a role is defined by its lens; the project expert becomes a domain knowledge pack (direction; design is post-first-run) | [stated] 2026-11-16 (general, #723) |
| D134 | #1124: no paid runs for now; the first smoke run is local-only and no spend ceiling is authorised | [stated] 2026-11-16 (1124, 1166) |
| D135 | #1123: Factory gets a top-level `lenses/` and `lens.json` | [stated] 2026-11-16 (1123) |
| D136 | #1123: the operator lens loads no knowledge file in v0; its prompt stays flat (D34) | [stated] 2026-11-16 (1123) |
| D137 | `rusty_orch lens breaker` is a read-only report with no reset | [default] 2026-11-17 (#758) |
| D138 | orch maps `sensitive_paths` to launcher `hide` via agents.json, and `real_data` to `bbp dry-run` | [default] 2026-11-17 (#758) |
| D139 | A lens post with no taint entry reads as `sensitive` (unknown ids from lost posts) | [default] 2026-11-17 (#758) |
| D140 | One paid lens run approved for 2026-11-17 (`--max-passes 5`, default call cap); supersedes D134 for that run only | [stated] 2026-11-17 (1124) |
| D141 | Run 1 toy `sweep` smoke run stays local-only; run 2 SSH host launcher brief is paid (codex now, claude after its `no_overage` entry) | [stated] 2026-11-17 (1124, 1166) |
| D142 | #1092: `/srv/lel` paths stay `container_file_t` on BaileyAi; no relabel before run 1 | [default] 2026-11-17 (1092) |
| D143 | D127's "first run" is a clean #1166 pass (settled claim, inbox fires, `bbp verify` clean); the freeze holds until a rerun meets it | [stated] 2026-11-17 (general, 1166) |
| D144 | #1166: rejecting `executed` from lens replies at `post::parse` is a bug fix, not new design | [default] 2026-11-17 (1166) |
| D145 | #1166: the llama.cpp 500 on valid harmony output is a host/adapter issue, not design | [default] 2026-11-17 (1166) |
| D146 | #1166: an inbox-exercising seed is a test-fixture choice; thresholds and escalation rules unchanged | [default] 2026-11-17 (1166) |
| D147 | #1166: the llama.cpp rebuild and the Qwen reasoning budget are held until the rerun's per-pass log shows which slot failed (D145 fix held, not applied) | [stated] 2026-11-17 (1166) |
| D148 | #1166 smoke-2: candidates never reviewed (stale question view) is an orch bug fix, re-read the question with `claim_card` | [default] 2026-11-17 (1166) |
| D149 | #1166 smoke-2: the architect prompt must say a claim has no `evidence` field (use `refs`); a prompt bug fix | [default] 2026-11-17 (1166) |
| D150 | #1166 smoke-3: the architect prompt also says one claim post per reply, no evidence post; a prompt bug fix | [default] 2026-11-17 (1166) |
| D151 | #1166: the first-run bar is met through reviewer prompt wording (support only when the brief supports the claim); Q54 resolved for now | [stated] 2026-11-17 (1166) |
| D152 | #1166: no host change; rerun smoke-4 after the prompt fixes alone (D147 hold stays) | [stated] 2026-11-17 (1166) |
| D153 | #1166: the Qwen reasoning budget (`--reasoning-budget 2048`) is the one host change allowed before smoke-5; the llama.cpp rebuild stays held | [stated] 2026-11-17 (1166) |
| D154 | #1166 smoke-4: Nano drops claim 6 (grounded score 0, deviates from the brief); Q has no accepted or chosen candidate | [stated] 2026-11-18 (1166) |
| D155 | Standing rule: every run's read-back includes the grounding audit before Nano acts on an escalated claim | [stated] 2026-11-18 (1166) |
| D156 | #1166 smoke-5: the architect prompt must say every claim sets `answers` to the question's id; a prompt bug fix | [default] 2026-11-18 (1166) |
| D157 | #1166: keep the D143 bar (criterion 3 not reworded); build D156 and rerun as smoke-6; the all-dropped BBP rule returns to Nano only if candidates keep dropping | [stated] 2026-11-18 (1166) |
| D158 | The grounding audit (D155) also covers candidates the board dropped, as a read-only report | [stated] 2026-11-18 (1166) |
| D159 | D143 criterion 3 for an audit-rescued candidate: one reopen, a lens run, then Nano's `choose` counts (D158 stays) | [stated] 2026-11-18 (1166) |
| D160 | #1571: Nano approves the agent launch profiles design as written (composition, env delivery, no service credentials in the jail, hooks deferred, build order) | [stated] 2026-11-18 (1571) |
| D161 | #1571 v0 isolation is `same_user` (agents run as Nano's user); `agent_user` once the BaileyAi host writes are approved | [stated] 2026-11-18 (1571) |
| D162 | #1571 profile files live in Factory (`factory/agent-profiles/*.json`, `factory/project-profiles/*.json`) | [stated] 2026-11-18 (1571) |
| D163 | #1166: reword the A2 fixture as "Proposed action, not run" (seed claim A stays the control); does not close Q53 | [stated] 2026-11-18 (1166) |
| D164 | Smoke-7 is the first run under D143; the D127 design freeze lifts; builder sessions (#1402, #600) are released first | [stated] 2026-11-18 (1166, general) |
| D165 | #600 builders: the Claude CLI in the v0 jail uses Nano's subscription (Q15) | [stated] 2026-11-19 (1318) |
| D166 | #600 v0: the Claude CLI reads Nano's `~/.claude` login through a read-only jail root (written exception to D101, `same_user` v0, expires with the gateway #726) | [stated] 2026-11-19 (1318, 600) |
| D167 | One paid run approved: the #1318 toy-claim acceptance run (2 builders, 45 min, one claim) after the Claude `no_overage` assertion is added; #1320 not approved | [stated] 2026-11-19 (1318) |
| D168 | #703: new bbp CLI surface `bbp human --decision ID` and `bbp decision` approved | [stated] 2026-11-24 (703) |
| D169 | #703: board-side API changes in PR #1844 (Escalations::act decision id, Escalations::applied, inbox_act `settled`) approved as built | [stated] 2026-11-24 (703) |
| D170 | #703: an uncertain inbox send is written ahead as a pending mark on the Remediate card; closed as not applied only on proof (SUPERSEDED by D171) | [stated] 2026-11-24 (703) |
| D171 | #703: Mill Board never closes a Remediate card from BBP's log; it only notes once the decision id shows applied (with rev); supersedes D170 | [stated] 2026-11-24 (703) |
| D172 | #703: PR #1844 surface approved as built, including required `--fence` on `bbp human --decision` (closes Q63) | [stated] 2026-11-24 (703) |
| D173 | #1416: the LEL tab in rusty_term reads BBP through the Mill Board HTTP API (inbox via `inbox_list`/`inbox_act`; new read endpoints for claims, intent and spend) | [stated] 2026-11-24 (1416) |
| D174 | #1416: the LEL tab view model and parsing live in rusty_term, fixtures from `bbp inbox --json` plus a drift test; no shared crate | [default] 2026-11-24 (1416) |
| D175 | #1092: the inbox runs on a second Mill Board on BaileyAi with its own SQLite file; rusty_tick retired, live board untouched | [stated] 2026-11-25 (1092) |
| D176 | #430: rusty_bbp's co-edit fold moves onto rusty_crdt's Rga as a pure swap; admission (overlap/stale refusal) unchanged, golden logs byte-identical | [stated] 2026-11-25 (430) |
| D177 | #1404 build defaults from the recovered #600 design: builders_per_claim 1, max_live_builders 3; budget admission as approved; `builder_session` fields | [default] 2026-11-25 (1404) |
| D178 | #1405: Windows-only intent lines are recognised by a frozen-input mapping file keyed by line; the check is orch's, no BBP change | [default] 2026-11-25 (1405) |
| D187 | #1318 v0 builder launch defaults: ACP permission allow_once + logged; usage 0 when unreported; `tail -F acp.log` viewer; mirror from frozen inputs; no jail token before #1382 | [default] 2026-11-25 (1318) |
| D188 | #1318 slice design approved; a builder's candidate reaches BBP only on `end_turn` with the `.lel/submit` marker and a non-empty diff, else the session ends Failed | [stated] 2026-11-25 (1318, 600) |
| D189 | #1318 S5 `builder run` defaults: explicit launcher flags, holder deadline past orch's grace, `sh` fd-4 wrapper, no recover in `builder run`, built-in prompt, `claude-builder` seat and op ids, store opened per step | [default] 2026-11-25 (1318) |
| D196 | #724: the redrawn `factory-map.svg` is accepted as the Factory map for now | [stated] 2026-11-25 (724) |
| D198 | #1416 slice 2: claim field bindings, stage strip derivation rule, brief text as a plain input | [default] 2026-11-25 (#1416) |

## D1. Lenses, not stages

**Decision.** Roles (planner, tester, reviewer, security, operator) become critic lenses that attack one shared artefact at the same time.
**Why.** Factory roles exist because humans can't hold everything at once. For agents, the handoffs only add queues, latency and lost context. The perspectives are the valuable part, not the sequence.
**Rejected.** Sequential role pipelines, including the earlier Factory design of one Claude Code thread per role with approval gates.

## D2. Converge on claims, not conversation

**Decision.** Agents post structured claims and evidence. They do not exchange prose.
**Why.** Prose debate is a human limitation. Structured claims are cheaper, auditable, and can be executed where possible.
**Rejected.** Chat-style debate between agents. Abandoning text altogether: tokens remain the native interface across vendors, and claims stay human-readable for audit.

## D3. The board computes confidence

**Decision.** Confidence is computed by the board from the evidence, as a deterministic Bayesian-style update. Executable evidence outweighs opinion, and agent track records weight the evidence.
**Why.** Agents self-reporting confidence is ungrounded and can be gamed. A deterministic rule is testable and replayable.
**Rejected.** Agents asserting their own confidence. Majority votes.

## D4. Budget is the price of certainty

**Decision.** Allowances scale with the stakes and reversibility of each decision. Work stops early once evidence stops moving confidence. A global ceiling and a circuit breaker cap total spend (#598).
**Why.** A flat token budget overspends on trivial calls and underspends on one-way ones.
**Rejected.** A flat token budget per task (Nano's earlier framing). Spending the whole budget just because it exists.

## D5. Nano writes intent and judges exceptions

**Decision.** Nano writes the intent. The board escalates only irreversible actions, taste calls, deadlocks the evidence can't settle, and budget overruns.
**Why.** Nano said he mostly clicks approve without reviewing. A rubber-stamp gate adds delay and false assurance.
**Rejected.** An approval gate at each stage.

## D6. Keep dissent honest

**Decision.** Adversarial lenses run on different models. Each lens forms its view blind before seeing the others. Agreement must cite a test, property or measurement. A lens that finds nothing is pushed to look harder.
**Why.** Agents sharing a model and a context drift into agreeing with each other.
**Rejected.** One model playing several personas. "Looks good" counting as support.

## D7. Several blind candidates, including a rebel

**Decision.** Open questions go to several models blind. At least one candidate must deliberately depart from the known approach. A question with only one candidate counts as more uncertain. Attacks start only once several candidates exist, and target all of them equally.
**Why.** Nano raised the groupthink concern. If one lens proposes and the others only attack, everything anchors on the first idea.
**Rejected.** One research answer that everyone else attacks.

## D8. The board is the context

**Decision.** Lenses are stateless for each question. For every question, orch assembles: the cached intent and rules, the claim with its evidence and dependencies, relevant settled claims at one line each, and the lens's track record. Builders may keep a short session within one task. Cross-project lessons go in remind-me.
**Why.** This avoids context rot and self-anchoring, makes blind first passes enforceable by construction, and keeps cost low.
**Rejected.** Agents keeping long conversations. Resetting context only at phase boundaries.

## D9. Planning, building and testing are concurrent

**Decision.** Planning, building and testing are activities, not phases. They run together on every claim. The only ordering: intent comes first, irreversible actions last.
**Why.** Phases are a human scheduling device.
**Rejected.** A plan, build, test sequence.

## D10. Research: learn, don't inherit

**Decision.** A research phase runs between the intent interview and the prototypes, and a research lens stays active throughout. Prior art is evidence and a baseline to beat, never a template. Copying code is out. At least one first-principles candidate competes on the same evidence.
**Why.** Nano wanted lessons already learned to be used, without forcing the work into existing designs. The square peg applies to dedup algorithms as much as to the process.
**Rejected.** Starting from a blank page. Following existing tools' designs.

## D11. BBP is a pure protocol

**Decision.** BBP owns the board's rules: claims, evidence, transitions, thresholds, budgets and escalation. Roles, rosters, lenses and model choices live in orch. bbp-core stays independent of orch-core, and the two compose (2026-10-08).
**Why.** In October BBP had hard-coded four roles, which made it one pipeline dressed up as a protocol.
**Rejected.** Roles in BBP. Rebasing BBP onto orch-core's board.

## D12. orch is model-independent, via ports and adapters

**Decision.** orch core defines one port: given a lens's context and claims, return structured evidence. There is one adapter per provider: Claude, OpenAI/Codex, Gemini, local llama-swap. Config binds each lens to an adapter, a model, a tier and fallbacks. Adapters validate output and reject non-conforming replies. JARVIS is the face, not the engine.
**Why.** Model diversity only works if no single model owns the engine. Swapping a model should be a config edit.
**Rejected.** A single-vendor engine. JARVIS as the engine. Vendor logic in orch core.
**Later.** D34 replaced the planned Gemini adapter: v0 runs Claude, Codex and local models.

## D13. No agent-to-agent calls

**Decision.** Every exchange goes through the board.
**Why.** Direct calls would bypass blind first passes, the evidence rules and the audit trail.
**Rejected.** Peer messaging between agents.

## D14. Tool-using agents join through the board over MCP

**Decision.** Agents that use tools (Claude Code, Codex) connect to the board through an MCP server and post claims and evidence themselves. orch calls the simple lenses directly.
**Why.** Tool-using agents don't fit a single request and reply.
**Rejected.** Squeezing tool-using agents into the lens port.
**Open.** How this fits whichever executor D20 settles on.
**Status.** Decided by Nano 2026-11-04: yes, but not before v0 ships. See D60, which supersedes "Proposed".

## D15. Concurrent, event-driven evidence posting

**Decision.** The claims model is built on event-driven, concurrent evidence posting (BBP Phase 2). Any agent posts when it has evidence; subscribed lenses wake and read the delta. The control plane stays serialized.
**Why.** Turn-taking throttles the flow the claims model depends on.
**Rejected.** A turn-based loop. Designing claims on top of turn-taking.
**Prerequisite.** Confirm on main whether Phase 1 is live and how much of Phase 2a has landed (#593).

## D16. Value-driven scheduling is a shadow experiment

**Decision.** Try value scoring, the board asking targeted questions, and fork-and-collapse together, as one system. Run it in shadow mode against plain event-driven scheduling, comparing tokens, time to settle and decision quality (#596).
**Why.** "Loop or event" only answers when an agent acts, not what is worth doing. The value approach is unproven, so it must beat a baseline before it becomes the default.
**Rejected.** Treating loop versus event as the only options. Adopting value scheduling without evidence.

## D17. Build in the rusty_mill monorepo

**Decision.** BBP, orch and Mill Board stay where they are, and the cards land there.
**Why.** They already live in rusty_mill and are tested together. The cards change the contracts between them. There is no forcing function for extraction.
**Rejected.** A new Let Em Loose repo.

## D18. The dedup experiment gets a fresh repo and intent only

**Decision.** The experiment's agents get only the intent. They never see `rusty_fclone`. Studying public tools such as fclones is allowed; copying code is not. `rusty_fclone` is the control.
**Why.** The experiment has to test the process, not reproduce existing code.
**Rejected.** Running the experiment inside rusty_mill. Handing over `rusty_fclone` to rebuild.

## D19. The Factory repo holds process and config only

**Decision.** `baileyrd/Factory` holds the intent template, lens definitions and model bindings. No code.
**Why.** The process can change without a code release.
**Rejected.** Code in the Factory repo.

## D20. No fourth executor

**Decision.** rusty_mill already has three executors: the orch dispatcher with BBP, mill-runner (worktree plus rusty_sandbox), and the super-board robot. The lens runner extends or replaces one of them.
**Why.** A fourth would deepen the duplication the consolidation audit is trying to remove.
**Rejected.** A new standalone executor.
**Resolved 2026-11-02.** The reconnaissance found super-board already retired (2026-10-28), so two executors remain. The lens runner extends orch, next to the dispatcher on orch's existing model port. mill-runner is left alone.
**Reopened 2026-11-04 (D103).** Nano: "Option C [stated]. Agents get mux-hosted ptys. This deliberately reopens D20 and D66; log both as reopened, with this as the reason." (thread message cmsg_01WgyiM31RsSnZTgNuPX2VE42EMPL7EXicY3bTWURHCCkG (2026-11-04T23:51:22Z)). The text above stands until the D103 design is approved; that design must show D20's guarantees still hold with orch deciding and the mux hosting.
**Re-closed 2026-11-05 (D104).** orch is the only executor; the mux hosts ptys. The mux forks an agent only when orch asks and never kills one on its own; each `bbp mod` guarantee maps as in mux-hosted-agents-design.md §3. Nano approved it on decision card cmsg_01WgyiM31RsSnZTgNuPX2VE41VyDsxTHxmCrXAPxQnCpra, 2026-11-05T00:00:23Z.

## D21. Thin slice first, then let it build itself

**Decision.** The order is: the claims model, two or three lenses, a bare inbox, then the dedup experiment. Once that works, it helps build the rest.
**Why.** End-to-end evidence comes before polish, and the slice is the honest test of the idea.
**Rejected.** Building the UI first.

## D22. Ground truth sits outside the agents' loop

**Decision.** Holdout tests are written from intent by a lens that builders never see. A small golden set is signed off by Nano. Calibration is tracked: claims accepted at about 90% should hold about 90% of the time (#597).
**Why.** Agents producing and judging their own evidence can be confidently wrong. A false 92% is worse than an honest 50%.
**Rejected.** Trusting agent-written tests alone.

## D23. Defer existing code and the smaller gaps

**Decision.** Existing code, sandbox and data boundaries, intent versioning and runaway cost go on #598. Lessons, gameable track records, maintenance and replay go on #599. Neither blocks v0, but existing code is the biggest risk for real projects.
**Why.** Further gap-hunting had diminishing returns. The remaining gaps will show up when the system runs.
**Rejected.** Designing every gap before building anything.

## D24. Audit remind-me before building

**Decision.** Audit the existing `project` field and the `remind_me_project` tool for active-project filtering before building anything new (#592).
**Why.** The capability may already partly exist. Writes from the remote connector come back with `project: null`.
**Rejected.** Building a new mechanism straight away.

## D25. Design first on every card

**Decision.** Each card posts a design or report for Nano's approval before any implementation. Schema and API changes are flagged.
**Why.** This matches Nano's working agreement, and these contracts are expensive to change.
**Rejected.** Implementing straight from the card text.
**Changed for M1 2026-11-12 (D130).** Nano approved M1's design steps (#1092, #1123, #1124, #1166) as one batch of runbooks and config, so those cards need no per-card design approval. D25 stands for every other card.

## D26. The name is Let Em Loose

**Decision.** The project is called Let Em Loose. A suggested tagline: loose, but on evidence.
**Why.** Nano chose it.
**Rejected.** "Beyond the Software Factory", which is kept as the concept doc's subtitle.

## D27. Claims sit beside the stage lifecycle

**Decision.** BBP's task lifecycle (planning, build, test, review) is itself the factory pipeline. In v0, claims are an opt-in mode per task: claims-mode tasks skip the stage lifecycle, and stage-mode tasks are unchanged. Evidence posts need no turn; the one-live-turn rule stays for state changes, budgets and approvals.
**Why.** Additive, so nothing existing breaks. Retiring the lifecycle is a later, separate decision once claims have proven themselves.
**Rejected.** Replacing the lifecycle in v0. Lifting the one-live-turn rule for every write.

## D28. Replay determinism, not order independence

**Decision.** The guarantee is that the same log always replays to the same outcome. Settling depends on arrival order, which the log records. Settled claims keep taking evidence: an executed contrary result reopens a claim into a fresh epoch, where it must re-earn its settlement, and a claim already acted on escalates instead. A passing run reopens a dropped claim the same way. A claim that has been superseded stays closed.
**Why.** Order independence could not hold once claims settle at a threshold. Refusing evidence after settling would stop holdouts and golden sets from catching a wrong settlement.
**Rejected.** Order independence. Refusing evidence once a claim settles.

## D29. Scoring and budget defaults

**Decision.** Confidence is a whole-number score.

| Setting | Value |
| --- | --- |
| Accept | 8 (90%) for low and medium stakes; 12 (about 96%) for high stakes and irreversible claims |
| Drop | -8 (10%) |
| Executed test | pass +4, fail -8; strength fixed at 1 |
| Cited source | weight 2, through a `source` artifact |
| Opinion | can never reach accept |
| Authors | at least 2 distinct authors to accept |
| Stuck | the last 6 evidence posts moved the score by 2 or less in total: escalate |
| Budget unit | one evidence post or one candidate run |
| Budget per claim | 8, 16 or 32 units for low, medium or high stakes; doubled if irreversible |
| Track records | recorded, not used as weights in v0 |

Storing a candidate that names a claim starts its run, which costs one unit and runs only in the sandbox. `sole_candidate` is a fact recorded in BBP; what to do about it is orch's decision.
**Why.** Two independent passing tests reach accept; one counterexample weighs as much as two passes, because a failure falsifies and a pass only supports. High-stakes claims need a third test.
**Rejected.** Probability floats with author-rated strength (one author could make a failure worth -24). A single accept line for every stake.

## D30. A failing test blocks acceptance

**Decision.** Automatic acceptance is blocked while failed profiles remain for the current candidate in the epoch; explicit human `accept` or `choose` can override this block under C-R8. Only a passing run that includes the failed profiles clears it. The host freezes the profile set when a task opens, in a host-only file, and records its hash in `task_opened`; the runner and the report check refuse any mismatch. A new candidate inherits the block until its own passing run includes those profiles. A reopened claim starts out blocked. A candidate whose diff changes or deletes listed test files cannot clear a block unless a different principal endorses the test change with cited support.
**Why.** Without the block, enough passes from other authors could accept a claim while a real failure stood. The rest closes the ways around it: dropping the test from the rerun, editing the profile, or editing the test.
**Rejected.** Score-only treatment of failures. Tracking individual tests, which would need a report schema change.
**Known limit.** Rust unit tests inside source files are not caught by the path check. The holdout suite (D37) covers this.

## D31. Superseding a claim

**Decision.** Any claim can be superseded, open or settled. The replacement takes effect only once the new claim is itself accepted; until then both stay live. Superseding a claim that was already acted on needs Nano's accept in the inbox, and the action record is kept.
**Why.** A badly worded open claim needs to be replaceable, but posting a replacement must not let an agent escape a failing test.
**Rejected.** Immediate supersession. Superseding settled claims only.

## D32. When a dependency collapses

**Decision.** Dependents that nothing has acted on reopen automatically. If the dependency was dropped, those dependents are voided in the same step and stay visible, so they can be re-proposed with new dependencies. Only dependents already acted on escalate. A new claim cannot depend on a dropped one.
**Why.** Escalating every collapse would flood the inbox with decisions that don't need Nano.
**Rejected.** Escalating every collapse. Automatically voiding claims that were already acted on.

## D33. Action marks and held reruns

**Decision.** Before acting on a claim, the host records a mark ("about to merge X") against the epoch it saw. If contrary evidence got in first, the mark is refused and nothing happens. A marked claim counts as acted on even if the host crashes; a mark with no recorded outcome after 1 hour (configurable) escalates for reconciliation. When the budget is spent, a rerun requested by an agent is refused, and one requested by a human becomes a budget escalation in the inbox. The old run stays valid either way.
**Why.** Without marks, a merge racing a failing test could lose the record that the claim was used. A human should never be silently blocked by a budget.
**Rejected.** Acting without a mark. Refusing human reruns outright.

## D34. Lenses, providers and local models

**Decision.**
- **Providers in v0:** Claude, Codex and local. The hard-coded provider list in orch core becomes a config-driven registry. A new `orch-openai` crate (one crate per adapter, as orch already does) talks to llama-swap, and plain HTTP is allowed only to loopback and private addresses. Gemini joins later through the same adapter over HTTPS. Per ADR-0019, old provider variants decode to IDs without rewriting stored history; new snapshots, including a resumed goal saved again, write `Id(String)` under schema v1. Older binaries cannot read those new snapshots and report them corrupt. Rollback therefore means not resuming, or discarding, goals saved by the newer version; read compatibility is not write/rollback compatibility.
- **Lenses:** each new question gets three blind Architect proposals on three models (Claude, local, and a Codex rebel; which model is the rebel is config). Research, Adversary and Operator follow. The lens asked about a claim rotates across the eligible lenses and models, and is never the claim's author.
- **Local models:** Operator on Qwen3.6-35B-A3B; gpt-oss-120b as the local high tier for sensitive or contested claims; 32k context to start.
**Why.** The Gemini CLI was discontinued, and every existing adapter is a CLI. One OpenAI-compatible adapter covers llama-swap now and Gemini later. Rotation and author exclusion keep the old "the reviewer is never the builder" rule.
**Rejected.** A Gemini adapter in v0. A fixed lens per kind of question.

## D35. The inbox is a read view over BBP in Mill Board

**Decision.** The inbox is a read view over BBP served by Mill Board. Nothing is copied into rusty_tick, because an escalation is a decision, not a task (ADR-0012). Mill Board reads only through the host's `bbp` command, never BBP's files, so in v0 it runs on the orch host. Each card shows the claim, confidence, budget, reason, the top two supporting and contrary evidence items, and a dry run computed by the pure engine. Actions:
- **Approve:** behavior depends on the escalation reason, as `plan` and `plan_task` specify. A claim-budget approval adds one `base(c)` (stakes and reversibility included), then reopens the claim into a fresh epoch; it does not accept it or clear failed profiles. A held rerun approval adds one `base(c)` and starts that rerun. A whole-task approval adds each exhausted total's own opened allowance to its current limit, then resumes the task; it uses no claim-tier amount and accepts no claim. Totals already replenished on a partial retry are not extended again. An irreversible-action approval accepts without adding budget. An unconfirmed action's Approve records the mark's outcome as `done`.
- **Reject:** drops a claim, withdraws a held rerun, records an unconfirmed action as `aborted`, or cancels a whole task with a required reason, according to the escalation. Dropping an acted-on claim also creates a "Remediate" card carrying the dry-run diff.
- **Choose:** for a stuck question with candidates (including a taste call), Nano picks one; the chosen one is accepted and the rest dropped, in one step.
- **Refine intent:** records a human clarification linked to the displayed brief, then reopens the claim; for a paused whole task it extends exhausted totals and resumes instead. Recorded so it can become an intent version under #598.

Every preview and mutation uses the task revision read for the displayed escalation. The CLI adapter supplies `--rev` and advances the expected revision only after each committed step; stale work is refused and must be re-read/previewed. Approval concerns that exact claim/action snapshot and its displayed brief, candidates, evidence and dry-run result. It is not append-only confidence evidence or blanket permission for later actions or a refined intent. C-R8 human acceptance and C-R9 epoch-fenced action marks remain distinct from executing an external action. Formal intent versioning is still #598; this guide does not claim a new intent-version field or separate dry-run signature exists.

No push notifications in v0. Sorting uses BBP facts: irreversible first, then oldest.
**Why.** One source of truth, and Mill Board already has admin gates, MCP and a web UI.
**Rejected.** Storing escalations in rusty_tick (this reverses the earlier recommendation in QUESTIONS Q21). Approve and Reject only.

## D36. Value scheduling lives in orch, with a pre-registered rule

**Decision.** The value score, "the board asks" and fork-and-collapse all live in orch behind a `Policy` trait, with no BBP change. Fork-and-collapse only starts after #600, on reversible claims, with two sandboxed branches capped at 2 units and 30 minutes each. On a tie, the simpler or cheaper branch wins and the reason is recorded; it goes to Nano as `choose` only if the intent marks it as taste. Evaluation runs in shadow mode first, then paired runs on five intents. Value scheduling becomes the default only if (a) its holdout result is no worse on all five (at most one more failing holdout test than the baseline), (b) it uses at most 80% of the baseline's tokens on at least four of five, and (c) total escalations are no higher than the baseline.
**Why.** The score is a scheduling policy, not a board rule, so it can change without touching replayed logs. Five runs is a small sample, so quality and Nano's time are not traded for savings.
**Rejected.** Exposing the score from BBP for sorting the inbox. A rule that allows worse quality on one intent.

## D37. Holdouts, protected tests, golden sets, calibration

**Decision.**
- **Holdouts:** a second frozen profile suite, written from the intent alone by a holdout lens on a model no builder uses (config is refused otherwise). Only the holdout lens can run it, on the same base and diffs as the current candidate. For a task opened with holdouts, automatic acceptance of a claim with a candidate also needs a passed holdout run bound to the current candidate tree and suite digest; explicit human accept/choose remains the override. Builders see only profile names and pass or fail counts.
- **Protected test paths:** a candidate whose diff touches one is refused, with the paths named. A real test fix goes in as its own `tests` claim, which a different lens must support. This replaces the interim rule in D30.
- **Holdout versions:** a new version needs Nano only when an existing holdout test is removed or changed; pure additions are adopted automatically.
- **Golden sets:** in the Factory repo at `golden/<intent>/`, with a `MANIFEST` and `SIGNOFF` containing `digest`, `signer` and `date`. As **Golden sets** in the claims addendum specifies, the host checks the manifest's file hashes and the signoff's exact set digest, refusing `bad_golden` on mismatch. This is human digest-attestation: Nano's endorsement must be obtained through the human review process. CODEOWNERS routes reviews and may require them when branch protection is configured; it does not prevent another writer creating the file or authenticate its author. The host's digest check detects changed content or a stale mismatched signoff, but neither file presence nor a matching copied/forged signer label proves Nano's identity. Cryptographic signer authentication is not part of the accepted v0 contract. Private fixtures from Nano's drive are recorded as hashes only and mounted read-only. Golden cases run as `golden:` profiles inside the holdout suite.
- **Calibration:** orch-store records each accepted claim as held, reversed or unknown, at a 14-day horizon, with a hold rate per score band and nothing proposed below 20 claims per band. It is informational in v0. `rusty_orch calibrate` proposes changes that Nano approves, and they apply only to tasks opened afterwards.
**Why.** Agents that write and judge their own tests can be confidently wrong. Quietly restoring tests would hide the fact that an agent tried to change them.
**Rejected.** Restoring protected paths from base. Using track records as weights in v0.

## D38. Phase B intents and spend

**Decision.** The five Phase B intents are `dedup-lite`, `tick-lite`, `vt-grid`, `gate-board` and `frag-check`, each a cut-down version of an existing project. `dedup-lite` is a benchmark, not the #600 experiment. Acceptance lines describe observable behaviour, never method: for example, "two concurrent writers never corrupt the store and neither write is lost", not "temp file then rename". The `dedup-lite` timing target is 2 seconds on Nano's desktop, warm cache, median of 5, both arms on the same host. Taste domains: `cli-output` in tick-lite and `api-shape` in gate-board. Planning ceilings, pending separate spending authorization: $200 in total for Phase B, a $25 hard cap per run, and a pause to report if the first paired intent costs more than $40. These ceilings and BBP budget extensions authorize no billed provider use. ADR-0021 rejects any nonzero configured v0 rate; subscription calls also require the operator-controlled no-overage assertions and host admission checks linked above. A ceiling does not bypass admission, authorize a live call, or permit enabling credits/overages.
**Why.** Small, checkable intents with exact reference implementations keep a five-sample comparison meaningful.
**Rejected.** Method-prescribing acceptance lines. Swapping `vt-grid` for a fuzzier intent.

## D39. remind-me gets an explicit project parameter

**Decision.** `remind_me_add`, search and capture get an explicit `project` parameter, and the remote connector sends it. The proposed write to existing records is on hold until Nano has seen exactly which records it changes.
**Why.** The project was guessed from the caller's working folder; the remote connector has none, so it came out null.
**Rejected.** Guessing the project from other fields.

## D40. Build the slice before designing more

**Decision.** The reported build order after #597 was the #593 engine first, test-first from the claims traces, on its own PR. This is a historical sequence, not an instruction or permission to start implementation. Check current main and the separately approved per-card contracts before selecting work; #598 and #599 were not designed at this concept checkpoint.
**Why.** They don't block v0 (D23), and the slice will show which gaps actually matter.
**Rejected.** Designing every card before building any.

## D41. Gate on code erosion, not only behaviour

**Decision.** A small, pure metric, `rusty_erosion`, scores two things in basis points: how much cyclomatic complexity sits in functions above CC 10 (concentration) and how many tokens sit in repeated, normalized 50-token windows (duplication). It runs as an `erosion` profile in the task's frozen profile set and compares the candidate with the pinned baseline, not the absolute level. A candidate over the limit fails that profile, so the existing failing-test block (D30) stops acceptance. There is no BBP schema change. A claims task opens only with an erosion profile or a recorded opt-out, both covered by the profile digest.
**Why.** Every other gate checks behaviour, so a candidate could pass them all while the code got worse. SlopCodeBench (arXiv 2603.24755) found that happens in about 3 of 4 agent trajectories, and that quality prompts lower the starting point but not the rate of decay. Measuring against the baseline means existing code is not punished for mess it already had.
**Rejected.** Relying on the Architect lens prompt alone. A new BBP evidence kind for erosion.

## D42. Erosion limits come from our own history

**Decision.** The limits are calibrated from erosion deltas on rusty_mill's merged human PRs. During report-only, the profile always passes and the deltas still go in the log. Report-only lasts 2 weeks or 10 tasks, whichever comes first. At expiry, if thresholds are not approved, pause opening new affected tasks and send the calibration numbers to Nano for approval; do not extend report-only indefinitely or invent limits. Tasks already open retain their settings, frozen profile definitions and profile digests, including report-only behavior if that is what they opened with. Approved thresholds apply to newly opened tasks. The expiry behavior is the recorded owner choice linked above.
**Why.** Fixed numbers would be guesses. The repository's own human history is the fair baseline. The time box stops report-only from becoming permanent.
**Rejected.** Starting with fixed limits.

## D43. No erosion-specific override; budget approval is separate

**Decision.** v0 adds no erosion-specific override mechanism. A better candidate with a qualifying passing run can clear the failed profile. If its claim budget is exhausted, inbox budget Approve funds another attempt and reopens the claim (D35); the failed-profile block remains. Overriding the failure instead requires a separate explicit, revision-fenced human `accept` or `choose` under C-R8, with the failing profiles visible. Budget approval alone cannot do that. An erosion escalation shows the change in each metric and the three worst-affected functions.
**Why.** The existing human acceptance override needs no BBP change; it is a different action from extending a budget. The deltas and the worst functions let Nano judge without opening the diff.
**Rejected.** An `erosion` claim domain endorsed by another principal, like `tests` (D37). That stays available for v1 if overrides become common.

## D44. Scheduled cleanups pay erosion down

**Decision.** orch writes a `consolidate` intent when the cumulative erosion since the last cleanup reaches half the limit, or after 5 accepted claims, whichever comes first. A cleanup must keep the frozen suite and the holdouts (D37) green. The owner-approved cleanup predicate is `delta_concentration <= 0` and `delta_duplication <= 0`, with at least one delta strictly negative. One metric must improve and neither may worsen; requiring both deltas to be negative would be impossible at a zero floor. If both baseline metrics are already zero, skip the scheduled cleanup and record the reason rather than open an impossible task. This cleanup predicate is separate from ordinary calibrated erosion limits and does not change existing tasks' frozen profiles.

Worked examples (concentration, duplication, in basis points):

| Baseline | Candidate | Cleanup outcome |
| --- | --- | --- |
| (100, 0) | (80, 0) | Pass: concentration improves, duplication stays at zero |
| (0, 100) | (0, 80) | Pass: duplication improves, concentration stays at zero |
| (100, 100) | (80, 101) | Fail: duplication worsens |
| (100, 0) | (100, 0) | Fail: neither improves |
| (0, 0) | — | Skip cleanup; both are already zero |

**Why.** Prompts don't slow the decay. Human repositories decay less because people refactor, so the schedule makes that habit explicit and gives it a budget.
**Rejected.** Waiting until a gate blocks before cleaning up.

## D45. Rust only; unmeasured code is reported, not failed

**Decision.** v0 measures Rust only. Other languages are added when a real task needs them. Non-code files (md, toml, yaml, json and similar) are ignored. Code in an unsupported language is marked `unmeasured` on the claim and in the report, and counted in calibration, but it does not fail the profile or escalate. Known limit: the metric can be gamed, for example by splitting functions or renaming. It is a heuristic, and calibration watches for gaming, such as function count or total CC rising while concentration falls.
**Why.** No speculative language support. Recording `unmeasured` keeps the blind spot visible without sending every mixed-language change to Nano.
**Rejected.** Treating unmeasured code as a failure. Supporting Python before a task needs it.

## D46. Three-tier escalation for design questions

**Decision.** A design question is answered when the record (this file, the handoff pack, remind-me, Nano's standing principles) settles it; implementation detail gets a logged `[default]`; only intent, scope or taste, or anything that would reverse a `[stated]` or `[agreed]` decision, goes to Nano.
**Why.** Nano was relaying design questions between Claude sessions by hand.
**Rejected.** Asking Nano whenever a thread is unsure.
**Tag.** [stated] · remind-me: mem_8d9722c4207840afa0df9e187897055d
**Source.** Nano's handoff prompt, 2026-11-03.

## D47. Ask the project expert before Nano

**Decision.** Threads and the orchestrator consult the `let-em-loose-expert` subagent before asking Nano any Let Em Loose design question.
**Why.** The expert holds the design context; Nano only sees what it can't answer.
**Rejected.** Threads asking Nano directly.
**Tag.** [stated] · remind-me: mem_9645385181204711adf0e4bb1b035c4d
**Source.** Nano's handoff prompt, 2026-11-03.

## D48. Never invent a decision

**Decision.** Anything that can't be sourced goes in `handoff/OPEN_QUESTIONS.md`, not into an answer or a decision.
**Why.** A made-up decision looks settled and is worse than an honest gap.
**Rejected.** Filling gaps with plausible answers.
**Tag.** [stated] · remind-me: mem_6c172130fa684069ba0f11f1cca63916
**Source.** Nano's handoff prompt, 2026-11-03.

## D49. The handoff pack is scoped to the project folder

**Decision.** The pack lives in `docs/projects/let-em-loose/handoff/`, the subagent in `.claude/agents/let-em-loose-expert.md`, and the CLAUDE.md rule applies only to Let Em Loose paths and cards.
**Why.** rusty_mill is a monorepo hosting many projects; the v1.2 template assumes one project per repo.
**Rejected.** `docs/handoff/` at the repo root, and a generic `project-expert` name.
**Tag.** [default] · remind-me: mem_5eae422a1d394844951db4a0446cf92c

## D50. One decision log, D-numbering continues

**Decision.** This file stays the only decision log. `handoff/DECISIONS.md` points here and new entries continue at D46.
**Why.** Two logs would drift.
**Rejected.** Copying D1–D45 into the pack.
**Tag.** [default] · remind-me: mem_7c60a2deca2b4004a90a58f221fb518d

## D51. The expert is the single writer of settled answers

**Decision.** Callers pass Nano's answers back to `let-em-loose-expert`, together with where he said them. The expert writes one `remind_me_add` and then one entry here, both carrying a stable key, and ticks `handoff/OPEN_QUESTIONS.md`. Callers never write either.
**Why.** A single writer with a key keeps each answer recorded once. This is best effort: concurrent sessions or a crash between the two writes can still duplicate an answer, and the key is how the next run reconciles it (#1445 review).
**Rejected.** Each thread recording its own answers.
**Tag.** [default] · remind-me: mem_df536a0f908b40db9fdb87196ba62e35

## D52. rusty_erosion location and core/adapter module split

**Decision.** The crate lives at `crates/tools/rusty_erosion` (`[package.metadata.rusty_mill] layer = "tools"`, added to the workspace `members` next to `crates/tools/rusty_baseline`). `src/core/` is the pure domain core: no I/O and no `tree_sitter` import. `core/mod.rs` holds the input types (functions with path, name and CC; token streams) and the integer basis-point results, `core/concentration.rs` the CC-above-10 share, `core/duplication.rs` the normalized 50-token-window share. `src/adapter/` is the tree-sitter side: `adapter/mod.rs` classifies files (Rust, non-code ignored, other code `unmeasured`) and skips `#[cfg(test)]` and `tests/`; `adapter/rust.rs` parses with `tree-sitter-rust` into the core's types (CC per function, identifier- and literal-normalized tokens). The `erosion` bin (`src/main.rs`: `git cat-file` reads and the baseline compare) comes with the profile slice, not slice 1.
**Why.** `rusty_baseline` is the nearest analog, a measurement lib plus bin under `crates/tools`. The split mirrors the design's "pure core plus a tree-sitter adapter" (erosion-gate-design §2), keeps the core testable on fixed fixtures (§10), and lets #705 add `adapter/python.rs` without touching the core. A path or module move is cheap to reverse.
**Rejected.** `crates/libs/` (no existing family fits; it is a tool run as a profile). Two crates for core and adapter (more workspace churn for no consumer). Putting it under `rusty_bbp_host` (the metric is independent of the host).
**Tag.** [default] · remind-me: mem_9d2be473f22f458a89532eac5e3b6363
**Key.** lel:704:rusty-erosion-layout
**Source.** Caller question from the #704 slice 1 build thread, 2026-11-03; erosion-gate-design §2, §9, §10; card #704 Build 1.

## D53. One host: everything deploys on BaileyAi

**Decision.** The BBP host, claims runs, shadow-run tooling, the daily watcher, `orch-runner` and the `lel-codex` container all run on BaileyAi. The agentos container on the MS-01 (LXC 107) is dropped. Supersedes the card #598/#599 note of 2026-11-02 22:08Z (mem_f7c9785174124786b33031d3cc1407bc).
**Why.** Nano resolved the conflict between that card note and his 2026-11-03 voice answer by choosing BaileyAi, where llama-swap already runs.
**Rejected.** agentos on the MS-01 with BaileyAi as fallback.
**Tag.** [stated] · remind-me: mem_5862c484286b46c9a3ff25d6ff2fde7e
**Key.** lel:general:one-host-baileyai
**Source.** Nano's saved record (cap_58e514f9f3d94b3da5d26c0b8a118dd3; dialog mem_77a1f7b27e1340d8a644b8b2062b8f6d, 11-03 "Nano chose all on BaileyAi"; summary mem_eb16b0dea2394f46990c384f69a2ac41 "all deployment on BaileyAi"), pointed to in thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4E2HeirtwhX9Qh6porRyDds (2026-11-04). Staged as R1 in PR #1446.

## D54. Lens shell reaches BBP through one `bbp mcp` session per lens

**Decision.** The `rusty_orch lens run` shell spawns one `bbp mcp` stdio JSON-RPC child per lens for the run (`BBP_SESSION=yes`, `BBP_PRINCIPAL=<lens id>`) and uses its claims tools: `claims_delta` to wake, `claim_card` and `claim_line` for the card and settled lines, `post` for claims and evidence. `claim_card`'s `EvidenceView` carries author, direction, kind, delta, epoch and scoring but no note, so the shell fills each evidence note from an incremental `read(after)` on the same session, cached by message id (one read per wake, not per claim).
**Why.** Design rev 3 §5 makes each lens a BBP principal with one session credential, and §6 steps 1 and 7 call `claims_delta` and post with that credential; `bbp mcp` is the only existing credentialed claims path, so this needs no BBP change and keeps the session's `read_messages` filtering. A child process per lens is cheap to replace.
**Rejected.** New finite `bbp` CLI subcommands for claims views and lens posts (new public CLI surface, ask-first). Linking `rusty_bbp` and opening `FsStore` in-process (bypasses the host's credential and visibility path). Adding `note` to `claim_card` (public API change to the addendum's card, ask-first; open question).
**Tag.** [default] · remind-me: mem_fc34b139927341aabc53a249d2427f81
**Key.** lel:594:lens-shell-bbp-transport
**Source.** Caller question from the #594 shell slice, 2026-11-04; 594-lens-runner-design.md §5, §6; bbp-claims-addendum.md § Reads, delta and subscriptions and R11; `rusty_bbp/src/claims.rs` `EvidenceView`/`ClaimView`; `rusty_bbp_host/src/mcp/claims.rs`.

## D55. Orch-originated intents are parked

**Decision.** Neither the narrowed v0 nor the full design in orch-intents-design.md is approved. D44 cleanups open as already planned: orch writes the `consolidate` intent at half the drift limit or after 5 accepted claims, with no origin header, no `rules.toml` modes, no host rule check and no `would_open` table. Revisit when a second task-opening rule appears. The implementation defaults proposed for that design (key `lel:general:orch-intents-defaults`) were never adopted and don't apply.
**Why.** Only D44 opens tasks today. #597 re-checks, #599 maintenance and #598 seeding all post into tasks that already exist, so an origin/mode mechanism would have one user.
**Rejected.** The narrowed v0 (origin only for `consolidate`, through a separate host entry with modes off|auto). The full design (general origin header, rules engine, observe mode with a `would_open` table and `rusty_orch rules report`).
**Tag.** [stated] · remind-me: mem_8727494da22f471998b4df97302c8de6
**Key.** lel:general:orch-intents-parked
**Source.** Nano chose "Neither, park it" on a decision card in thread "Discuss Let Em Loose design", 2026-11-03T22:54:11Z; mem_b6859bf1682b4b8c804edc23342f6746 (dsh-superagent review).

## D56. Windows-specific #600 behaviour is tested by a Windows shadow run and golden cases

**Decision.** Windows-specific behaviour of the #600 dedup intent (mapped drives, UNC paths) is covered by a read-only shadow run on Nano's Windows machine plus golden cases he signs off there. The Linux sandbox covers everything else. Each Windows-specific intent line must map to one of these, or it counts as untested.
**Why.** Claims runs execute only in the Linux sandbox, which can't exercise Windows path behaviour.
**Rejected.** Treating the Linux sandbox as enough for Windows paths.
**Tag.** [stated] · remind-me: mem_b33951241b4740a2b44f9cd73a09e2fe
**Key.** lel:600:windows-shadow-run-golden
**Source.** Nano's saved record (cap_58e514f9f3d94b3da5d26c0b8a118dd3; dialog mem_77a1f7b27e1340d8a644b8b2062b8f6d, 11-03 "a Windows shadow run plus golden cases for #600"; summary mem_eb16b0dea2394f46990c384f69a2ac41), pointed to in thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4E2HeirtwhX9Qh6porRyDds (2026-11-04). Staged as R2 in PR #1446.

## D57. #596 adds an erosion pass rule: no worse than baseline on 4 of 5 runs

**Decision.** The #596 pre-registered decision rule gains a fourth criterion: the treatment arm's erosion delta is no worse than the baseline arm's on 4 of 5 runs. It sits beside holdout 5/5, tokens ≤80% on ≥4/5 and escalations ≤ baseline (erosion-gate-design §6).
**Why.** Value scheduling must not win by writing messier code. Pre-registering it before Phase B runs keeps the comparison honest.
**Rejected.** Leaving erosion out of the #596 rule.
**Tag.** [stated] · remind-me: mem_bff2870445d746e7b1b11f34cd9bb88e
**Key.** lel:596:erosion-criterion
**Source.** Nano, thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4UtGWmEuydpHpmL6uyyvn7t (2026-11-04T11:58:44Z), answering the numbered list in cmsg_01WgyiM31RsSnZTgNuPX2VE48CSJZ2CFK5Xom7dXjiyN1X, item 1 "Yes". OPEN_QUESTIONS Q2.

## D58. #599 §6 schema and API changes approved

**Decision.** The changes listed in 599-gaps-design.md §6 are approved: BBP `claim_history` and its `Step` type (read-only, no log change); `bbp why --claim/--at/--task`; a `canary` run on a released ref; `rusty_orch lessons`, `rusty_orch watch` and `rusty_orch lens why`; orch-store tables `lesson_written`, `lens_record` (replacing #594's unimplemented counts) and `watch_finding`; the `Watch` port with `cargo-deny`, `cargo-update`, `toolchain` and `canary` adapters (CLI calls, no new crates); Factory `apps/*.json`, the watched-versions file and `watch_usd`. remind-me needs no change, but check before building that its HTTP add route carries `project`; an additive fix there is flagged then.
**Why.** All additive and dependency-free; this clears the ask-first schema/API gate for #599.
**Rejected.** Holding §6 for further review.
**Tag.** [stated] · remind-me: mem_69ede4b656794a0d8301818a68ce1957
**Key.** lel:599:section6-approved
**Source.** Nano, thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4UtGWmEuydpHpmL6uyyvn7t (2026-11-04T11:58:44Z), answering the numbered list in cmsg_01WgyiM31RsSnZTgNuPX2VE48CSJZ2CFK5Xom7dXjiyN1X, item 2 "Yes" (the question: "approve the new commands, tables and `Watch` port listed in its §6"). OPEN_QUESTIONS Q3.

## D59. #598 §6: task_paused, intent_revised and data classes approved

**Decision.** Approved from 598-gaps-design.md §6, as named in the question: the host write `task_paused { reason, detail }`, which moves the task to `escalated`; the human operation `intent_revised { brief, affects }` with the `intent_version` fold (omitted `affects` means `all`, an empty one is refused `bad_affects`); data-class labels, where `internal` may go to cloud models and `sensitive` stays local. The other §6 items weren't in the question; Nano approved them later the same day (D62).
**Why.** Schema and API changes are ask-first, so approval covers only what Nano was shown.
**Rejected.** Reading the "Yes" as approval of all of §6.
**Tag.** [stated] · remind-me: mem_eddc237646b843bd8069a0e7a86047a6
**Key.** lel:598:section6-named-items-approved
**Source.** Nano, thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4UtGWmEuydpHpmL6uyyvn7t (2026-11-04T11:58:44Z), answering the numbered list in cmsg_01WgyiM31RsSnZTgNuPX2VE48CSJZ2CFK5Xom7dXjiyN1X, item 4 "Yes". OPEN_QUESTIONS Q5.

## D60. Tool-using agents join over MCP after v0 ships

**Decision.** D14 is decided: tool-using agents (Claude Code, Codex) join the board through MCP, but not before v0 ships. This supersedes D14's "Proposed" status. How it fits the D20 executor stays open.
**Why.** They don't fit a single request and reply (D14), and v0 ships first on the lens port.
**Rejected.** Adding the MCP path before v0.
**Tag.** [stated] · remind-me: mem_c6c6304576114e5fa962e49e720e081e
**Key.** lel:general:mcp-agents-after-v0
**Source.** Nano, thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4UtGWmEuydpHpmL6uyyvn7t (2026-11-04T11:58:44Z), answering the numbered list in cmsg_01WgyiM31RsSnZTgNuPX2VE48CSJZ2CFK5Xom7dXjiyN1X, item 5 "Yes" (to "yes, but not before v0 ships"). OPEN_QUESTIONS Q6.

## D61. #708 alien review lens parked until v0 runs

**Decision.** The #708 alien review lens is parked until v0 runs: trigger policy, per-language defect lists, scoring and how reproduced findings enter BBP stay undesigned and unapproved, and nothing is built.
**Why.** Get v0 running before adding another lens kind.
**Rejected.** Designing #708 now.
**Tag.** [stated] · remind-me: mem_44c8bf81957240d8a0773acb5dc63f7f
**Key.** lel:708:alien-lens-parked
**Superseded.** By D74 (2026-11-04T20:16Z): Nano unparked #708.
**Source.** Nano, thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4UtGWmEuydpHpmL6uyyvn7t (2026-11-04T11:58:44Z), answering the numbered list in cmsg_01WgyiM31RsSnZTgNuPX2VE48CSJZ2CFK5Xom7dXjiyN1X, item 7 "Park it until v0 runs". OPEN_QUESTIONS Q8.

## D62. The rest of #598 §6 approved

**Decision.** Every remaining item in 598-gaps-design.md §6 is approved as listed, completing D59. BBP: `task_opened.baseline_digest` (optional); `claim_reopened.by: intent`; the escalation reason `intent_changed`. Host: `bbp open --base` runs the baseline and can refuse `baseline_red`; the `base_test_changed` refusal; named sandbox profiles for lenses and builders, not just runs; the filtered view; the `shadow:` profile kind with read-only `real_data` mounts and a plan artifact. orch: the Survey and Intent lens kinds; `max_class` and `price` on providers, both required at load; orch-store tables for item classes, spend and reservations; `rusty_orch lens breaker`; `domain_classes` replacing #594's `sensitive_domains`. Factory: `domain_classes`, `sensitive_paths`, `real_data`, `price`, the ceilings and the breaker settings. No migration: no claims-mode logs exist outside tests.
**Why.** All additive, and they implement answers Nano already gave on #598 (Q1, Q4, Q6). This clears the ask-first schema/API gate for the #598 build.
**Rejected.** Going item by item (option B on the card).
**Tag.** [stated] · remind-me: mem_fc5ad74f9fac448d8d22a9ff42f12bc3
**Key.** lel:598:section6-remaining-approved
**Source.** Nano tapped option A "Approve all as listed" on decision card cmsg_01WgyiM31RsSnZTgNuPX2VE4Ppm26TJdNSuETWT9FqZp8E ("Approve the rest of #598 §6, not just the three I listed?"), 2026-11-04T17:15:20Z. OPEN_QUESTIONS Q5.

## D63. #600's Windows GUI is judged only on Windows

**Decision.** The #600 desktop GUI (Windows-first) is judged only through the Windows shadow run and golden cases (D56). Linux claims runs check the engine and the web UI.
**Why.** Claims runs are Linux-only and can't exercise a Windows GUI.
**Rejected.** Judging the GUI in the Linux sandbox.
**Tag.** [stated] · remind-me: mem_0793274b6343443fbbe7f34d9846e0bb
**Key.** lel:600:windows-gui-judged-on-windows
**Source.** Nano, thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4MwQzj2Aawhe7Q5axWeGqTY (2026-11-04T17:16:45Z), "01 Yes. 02. Concur. 03. Concur with recommendation", answering cmsg_01WgyiM31RsSnZTgNuPX2VE497rAQtVYv9PGh6V7rgu8pq and 600-builder-session-design.md §11, O1 "Yes".

## D64. #600 builders are Claude CLI only in v0

**Decision.** In v0, #600 builder sessions run the Claude CLI only. Codex in `lel-codex` doesn't build.
**Why.** The holdout lens then keeps a model that no builder uses (D37), and isn't left with only local models.
**Rejected.** Codex as a second builder in v0.
**Tag.** [stated] · remind-me: mem_44f3236c741d4f3383370ac59299d664
**Key.** lel:600:claude-only-builders
**Source.** Nano, thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4MwQzj2Aawhe7Q5axWeGqTY (2026-11-04T17:16:45Z), "01 Yes. 02. Concur. 03. Concur with recommendation", answering cmsg_01WgyiM31RsSnZTgNuPX2VE497rAQtVYv9PGh6V7rgu8pq and 600-builder-session-design.md §11, O2 "Concur".

## D65. A Windows result that contradicts a Linux pass reaches the claim by Nano's reopen

**Decision.** When the Windows shadow run contradicts a Linux pass, Nano reopens the claim with a human `decision reopen` (C-R8), after `budget_extended` only if the claim's budget is exhausted (claims_engine.rs, cl_48). It isn't posted as `executed` evidence, since it has no runner report bound to the claim's candidate (C-R4).
**Why.** A real-data failure should stop acceptance, and only a reopen can do that. `cited` evidence never reopens a claim.
**Rejected.** Posting it as `cited` evidence.
**Tag.** [stated] · remind-me: mem_ed23f39443c2424d96b224556a54303f
**Key.** lel:600:windows-contradiction-human-reopen
**Source.** Nano, thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4MwQzj2Aawhe7Q5axWeGqTY (2026-11-04T17:16:45Z), "01 Yes. 02. Concur. 03. Concur with recommendation", answering cmsg_01WgyiM31RsSnZTgNuPX2VE497rAQtVYv9PGh6V7rgu8pq and 600-builder-session-design.md §11, O3 "Concur with recommendation" (option a).

## D66. The #600 builder-session design is approved

**Decision.** Nano approved the #600 builder-session design (`/mnt/project-files/let-em-loose/600-builder-session-design.md`) as a whole, together with D63–D65. Its §8 changes count as approved: the orch `Builder` job kind and its admission check; the config values `builders_per_claim`, `max_live_builders` and `builder_wall_secs`; the orch-store `builder_session` table; the open-time `untested` check for Windows intent lines; the orch-bbp builder launch profile with a pre-fetched dependency mirror; and a new local-only repo `dedup-manager` on BaileyAi. The #598 items it depends on are approved in D62. The §10 defaults (local-only repo, 45-minute wall clock, dependency mirror) stay `[default]`. #600 is built once #594 and #595 are in place.
**Why.** Design first on every card (D25). This closes OPEN_QUESTIONS Q7.
**Rejected.** Further changes before approval (the card's other option).
**Tag.** [stated] · remind-me: mem_c00104fb2fa0499c8f471bddcbb64cbc
**Key.** lel:600:builder-session-design-approved
**Source.** Nano tapped option A "Approve the design" on decision card cmsg_01WgyiM31RsSnZTgNuPX2VE4UkTT9YFWDhx1CsEYPh6YFg ("Approve the #600 builder-session design as a whole, with your three answers?"), 2026-11-04T18:08:52Z.
**Reopened 2026-11-04 (D103).** Nano reopened D66 together with D20 for mux-hosted agent ptys (thread message cmsg_01WgyiM31RsSnZTgNuPX2VE42EMPL7EXicY3bTWURHCCkG (2026-11-04T23:51:22Z)), quoted in D20. The builder launch path (the orch-bbp builder launch profile) is what the D103 design revisits; the rest of D66 stands until that design is approved.
**Re-closed 2026-11-05 (D104).** The #600 builder launch goes through the mux `Launch` (jail launcher as the block's process) in place of a direct spawn. Its lease, sandbox, wall clock and mirror are unchanged. Nano approved it on decision card cmsg_01WgyiM31RsSnZTgNuPX2VE41VyDsxTHxmCrXAPxQnCpra, 2026-11-05T00:00:23Z.

## D67. #722 drops the claim/evidence hash DAG; a linear per-line chain only

**Decision.** #722 v0 makes the BBP log tamper-evident with a linear chain in `rusty_bbp::fs_store`: each log line records `prev`, the SHA-256 of the previous line's exact bytes. The card's per-event hashes of the claims and evidence an event builds on (the DAG) are left out. They wait for the same trigger as signing: outside or untrusted agents joining.
**Why.** The chain's head already fixes every earlier event, and blob digests already fix evidence bytes. Per-event reference hashes only help prove one claim's trace to an outsider without the rest of the log.
**Rejected.** Building the DAG now ("Approve, but keep the DAG").
**Tag.** [stated] · remind-me: mem_5044b01631d14c9a9cba9b5e1dfa69b6
**Key.** lel:722:no-claim-evidence-dag
**Source.** Nano tapped option A "Approve all four as recommended" on decision card cmsg_014AqbZs58L565hMV2pFkCX59exuTYA3aBX2BcYCYExYmL ("Approve the #722 hash-chain design v1 as recommended?") in thread cmsg_014AqbZs58L565hMV2pFkCX5BMDsWTnkiWyUoTktyX3YZQ, 2026-11-04T20:23Z, as relayed by the coordinator. The recommendations are in /mnt/project-files/bbp-722/design.md § Questions for Nano, Q1.

## D68. #722: a broken chain fails every load

**Decision.** On every `load_log` (open, refresh, append), a chained line whose `prev` doesn't match the previous line's hash is `FsError::Corrupt`. The task stays unreadable until a human repairs the log. `bbp verify` still works on a broken log.
**Why.** This is how a corrupt line is handled today, and it follows the v0.5 spec rule that a reader never guesses. It also means `bbp why` / `claim_history` run over verified bytes.
**Rejected.** Reporting a broken chain only through `bbp verify`.
**Tag.** [stated] · remind-me: mem_39929b876bf243c082bba1c13068558b
**Key.** lel:722:broken-chain-fails-load
**Source.** Nano tapped option A "Approve all four as recommended" on decision card cmsg_014AqbZs58L565hMV2pFkCX59exuTYA3aBX2BcYCYExYmL ("Approve the #722 hash-chain design v1 as recommended?") in thread cmsg_014AqbZs58L565hMV2pFkCX5BMDsWTnkiWyUoTktyX3YZQ, 2026-11-04T20:23Z, as relayed by the coordinator. The recommendations are in /mnt/project-files/bbp-722/design.md § Questions for Nano, Q2.

## D69. #722: chained line framing, no BBP schema bump

**Decision.** New log lines are written as `{"prev":"<sha256 hex>","events":[..]}`. A line starting with `[` is a legacy, unchained line. A chain can start in the middle of an existing log, and a legacy line after a chained one is `Corrupt`. The BBP schema number is not bumped. An older build reports a chained log as `Corrupt`, so rolling back means not opening chained logs with an older build, as with D34 / ADR-0019. The same PR updates the contract text that promises byte-identical stage-task logs (addendum Mode section and CL-01, v0.5 spec §441).
**Why.** It fails closed, keeps `Rev` and the fold unchanged, and lets existing logs start chaining. A bump would cover only new tasks and add another schema number.
**Rejected.** A schema bump, and the card's additive serde-default event fields.
**Tag.** [stated] · remind-me: mem_ac35393946a246a1898c0bad259e057c
**Key.** lel:722:chained-framing-no-schema-bump
**Source.** Nano tapped option A "Approve all four as recommended" on decision card cmsg_014AqbZs58L565hMV2pFkCX59exuTYA3aBX2BcYCYExYmL ("Approve the #722 hash-chain design v1 as recommended?") in thread cmsg_014AqbZs58L565hMV2pFkCX5BMDsWTnkiWyUoTktyX3YZQ, 2026-11-04T20:23Z, as relayed by the coordinator. The recommendations are in /mnt/project-files/bbp-722/design.md § Questions for Nano, Q3.

## D70. #722 adds `bbp verify`

**Decision.** `rusty_bbp_host` gets a new public CLI command, `bbp verify --dir D --task T [--expect H]`. It reads the one log at `log_path_for` under the directory lock, without going through `FsStore::open`, and never modifies the file. It prints `{"lines", "unchained", "head", "partial"}`. It exits non-zero if the chain is broken, if a trailing partial line exists (reported and left in place, never truncated), or if `--expect H` is given and H isn't the hash of any line. So a head saved earlier still verifies after legitimate appends, while a truncation or rewrite that drops H fails (design v1 § Checks).
**Correction.** This entry first said `bbp verify` fails on "a head that differs from `--expect`", which is the design v0 wording. That was a transcription error, not a new decision: Nano approved design v1, whose § Checks has the ancestry rule above. PR #1485 review comment https://github.com/Rusty-Mill/rusty_mill/pull/1485#issuecomment-6643813583 flagged it.
**Why.** New public commands are ask-first, and this one lets whoever keeps a head check the chain against it.
**Rejected.** None was offered.
**Tag.** [stated] · remind-me: mem_ea09aa6ec67b4829bd69b2910c7c2978
**Key.** lel:722:bbp-verify-cli
**Source.** Nano tapped option A "Approve all four as recommended" on decision card cmsg_014AqbZs58L565hMV2pFkCX59exuTYA3aBX2BcYCYExYmL ("Approve the #722 hash-chain design v1 as recommended?") in thread cmsg_014AqbZs58L565hMV2pFkCX5BMDsWTnkiWyUoTktyX3YZQ, 2026-11-04T20:23Z, as relayed by the coordinator. The recommendations are in /mnt/project-files/bbp-722/design.md § Questions for Nano, Q4.

## D74. #708 alien review lens unparked (supersedes D61)

**Decision.** #708 is unparked, replacing D61. Next: finish the design (the value threshold on #596's score, per-language defect lists, scoring, and how reproduced findings enter BBP), run the egm Goldfish review, and bring the design to Nano for approval. Build and open a PR for dottiusmaximus only after he approves it. The design is not approved yet (D25).
**Why.** Nano chose this when asked whether his 20:11Z message "Spin off threads for each one with a design or partial design" (cmsg_014AqbZs58L565hMV2pFkCX59ZUnUW7emMuJZYMRh7Zqw9) covered #708.
**Rejected.** Keeping it parked until v0 runs (D61), and designing and reviewing it without building until v0 runs.
**Tag.** [stated] · remind-me: mem_80f2a194cf714d4586d27184e3522cc9 (supersedes mem_44c8bf81957240d8a0773acb5dc63f7f)
**Key.** lel:708:alien-lens-unparked
**Source.** Nano tapped option B "Unpark: design, review, then build" on decision card cmsg_014AqbZs58L565hMV2pFkCX59b5EzthujQuUWFyYCJrHdB ("Unpark the alien review lens (#708), which you parked this morning?"), 2026-11-04T20:16:21Z, thread cmsg_014AqbZs58L565hMV2pFkCX5PRArQZK27dSqRZa2ZYPeC1. Reopens OPEN_QUESTIONS Q8.

## D75. #708 alien review lens design v1 approved (shadow-only)

**Decision.** Nano approved the six points in §10 of `/mnt/project-files/let-em-loose-708/708-alien-lens-design.md`:
1. v1 is shadow-only and changes nothing in BBP. Posting findings to BBP is a separate v2 design that comes back to Nano, since it is a BBP schema change.
2. Q1: `min_value` is the Phase A 80th percentile of #596's value score. Until that exists, the lens runs on irreversible claims only; after that, a 1-in-10 random control sample is added.
3. Q2: the Rust defect-mechanism list in §4.
4. Q3: the ranking sort in §5.
5. Q4 (v1): findings stay in orch-store. The v2 BBP route returns as its own design.
6. The orch-lens port change `Question.shape: Shape { Posts(Allowed), Raw }` (§9.1), a public API change inside orch.
The build is the four PRs in §9, with `--alien` off by default. v2 gets designed only if §8's bar is met: at least 1 new regression per 20 hunts, at under 15% of the lens's spend on those claims.
**Why.** The Goldfish review of v0 (`708-goldfish-review.md`) found 3 blockers. Making v1 shadow-only removes them and leaves BBP unchanged until the lens shows its worth.
**Rejected.** v0's shape, where reproduced findings block claims through BBP right away.
**Tag.** [stated] · remind-me: mem_8088651b940e45a6a0c7bb6807c1d237
**Key.** lel:708:design-v1-approved
**Source.** Nano, "approved", thread message cmsg_014AqbZs58L565hMV2pFkCX5NbYhrp6skRZoUmd7dbZWP2 (2026-11-04T20:27:23Z), replying to cmsg_014AqbZs58L565hMV2pFkCX56Y25HTNtmVLdhS1nDojGpK, which asked him to approve §10. Thread cmsg_014AqbZs58L565hMV2pFkCX5PRArQZK27dSqRZa2ZYPeC1. Closes OPEN_QUESTIONS Q8.

## D76. `lens run` merges as a disabled slice until the accept-line source lands

**Decision.** PR #1467 (`rusty_orch lens run`) may merge with `lens run` refusing every invocation (`UNKNOWN_CLAIMS`) until BBP exposes the task's frozen accept line. The runner never assumes `ClaimsConfig::default()`. The accept-line source lands as its own small PR stacked on #1467. How BBP exposes it (`accept_at` on `ClaimView`, the whole `ClaimsConfig`, or an operator `--claims` file, bundled with Q12) is not settled here; it is open as Q14.
**Why.** dottiusmaximus on #1467 (comment 6635856528) asked for owner acceptance of a disabled slice or to treat the source as a merge-readiness dependency. Nano chose the disabled slice.
**Rejected.** Holding #1467 until the source lands. Assuming the default thresholds (accept_at 8 / strict 12) for tasks opened with custom `--claims`.
**Tag.** [stated] · remind-me: mem_120c56253ad241ce954eb5602baabf53
**Key.** lel:594:lens-run-disabled-slice
**Source.** Nano's tap on hearthbot decision card cmsg_01WgyiM31RsSnZTgNuPX2VE485HdWybMYAyN7zK64ZJGmR (thread cmsg_01WgyiM31RsSnZTgNuPX2VE44fHuHfPowdWG17Xr6c2MBA), 2026-11-04T19:46:52Z, option "Merge as disabled slice". Review and merge gates still apply.

## D77. `claim_card` carries each claim's accept line and each evidence item's note and strength

**Decision.** Nano chose a small additive `rusty_bbp` change: `accept_at` on each claim view and `note` and `strength` on each evidence item, so the #594 runner reads the task's frozen accept lines instead of assuming defaults (closes Q12 and Q14). Build detail, `[default]`: `ClaimView.accept_at: i64` is the claim's effective line, `ClaimRecord::accept_at(&book.config)` with strict already applied (one field, not both lines; the runner needs only the claim's own line, and the tier rule uses `accept_at / 2` of it). `EvidenceView` gains `strength: u8` and `note: String`; the existing `Sight` filter already omits unreadable evidence, so no new visibility rule. No log schema or `PROTOCOL_VERSION` bump: views are derived by the fold, not logged. The addendum's `claim_card` bullet (§ Reads, delta and subscriptions) and its Schema and API changes list gain the fields, citing this approval. With it, D54's `read(after)` note fill and D76's `UNKNOWN_CLAIMS` refusal are retired.
**Why.** The #1467 reviewer refused `ClaimsConfig::default()` for tasks opened with custom `--claims`, and no host interface exposed the frozen config. A per-claim line is the smallest change and is computed by the host, so the runner can't misapply the strict rule. The addendum card already lists strength, so that part closes a gap with the merged contract.
**Rejected.** The whole `ClaimsConfig` on the card (option b). An operator `--claims` file the runner can't check (option c). Both lines on every claim (redundant).
**Tag.** [stated] · remind-me: mem_506f809db3894df0b29d8992879f373c
**Key.** lel:594:claim-card-accept-line
**Source.** Nano's tap on "a) Add accept line to claim view", hearthbot decision card cmsg_01WgyiM31RsSnZTgNuPX2VE4Pksa1peJwFSFnWsRe6jtve (thread cmsg_01WgyiM31RsSnZTgNuPX2VE44fHuHfPowdWG17Xr6c2MBA), 2026-11-04T19:55:49Z; dottiusmaximus on #1467; `rusty_bbp/src/claims.rs` `ClaimView`, `EvidenceView`, `ClaimRecord::accept_at`; bbp-claims-addendum.md line on `claim_card`.

## D78. #708 alien hunt runs as a runner pass, outside the scheduler

**Decision.** Slice 4 runs the alien hunt as a separate pass in the lens runner instead of adding `Slot::Enumerate`, `Slot::Hunt(u8)` and `State.hunts` to the orch-lens scheduler (design §9.2). The saved `lens_states` scheduler record keeps its schema tag. orch-store holds the durable "already hunted this candidate" memory as an additive record under its own tag, or a per-(task, claim, candidate) hunt marker. It is written even when a hunt keeps no suspects or produces no probe, so an empty hunt is not run again. Eligibility, limits and the §6/§8 numbers do not change.
**Why.** This is an internal layout choice inside the approved design (D75), and Nano would see the same results either way. It is more reversible, because it avoids a schema-tag bump on a record that just merged. Probe rows (#1495) can't mark an empty hunt, so they alone are not enough.
**Rejected.** The §9.2 scheduler slots, which bump the `lens_states` tag. Probe rows as the only hunt memory.
**Tag.** [default] · remind-me: mem_a35f69e5146a4a51b93b1097e9608d5e
**Key.** lel:708:alien-pass-outside-scheduler
**Source.** let-em-loose-expert default, 2026-11-04, question 3 from the #708 build thread on slice 4. Numbered D78 because origin/main already has D76 and D77.

## D79. #599 watcher posts as a `watcher` roster role

**Decision.** `rusty_orch watch` posts each finding as a claim through a dedicated `watcher` role on the app's maintenance task. The role holds `post ∋ claim` and its own session credential, and every maintenance task's roster includes it. Because `min_authors` is 2, a watcher claim never accepts on the watcher's word alone.
**Why.** Under the claims contract, only a roster role with `post ∋ claim` posts claims, using its session credential (addendum C-R1, roster rules). The host writes only board events. The human channel posts `answer` and `decision`, never claims (C-R10, Agent API item 4). A separate role also keeps the watcher's posts apart from the lenses' posts and their track records.
**Rejected.** The host posting the claims (the contract doesn't allow it). The human channel posting them (same reason, and it would present machine findings as Nano's). Reusing a lens role (it would mix watcher posts into that lens's record).
**Tag.** [default] · remind-me: pending
**Key.** lel:599:watcher-role
**Source.** let-em-loose-expert, 2026-11-04, answering the #599 build thread (plan §4), from addendum C-R1, C-R10 and the claims roster rule.

## D80. #599 maintenance tasks are opened by the human and named in `factory/apps/<app>.json`

**Decision.** The human opens each app's long-lived maintenance task with `bbp open --mode claims`, and `rusty_orch watch` never opens a task. The Factory registry is one file per app at `factory/apps/<app>.json`, beside the lens config, with the fields `{ app, repo, task, released_ref, profile, watch_usd }`. `task` holds the maintenance task id, and `profile` names the profile the canary re-runs on `released_ref`. `watch` must not reuse `rusty_orch maintain`'s task-creation path.
**Why.** D55 says only D44 opens tasks and that #599 maintenance posts into tasks that already exist. D58 approves `apps/*.json` and `watch_usd`. Plan §4 lists the fields; `profile` is added because the canary needs it. A file per app is the easiest layout to change later.
**Rejected.** `watch` opening its own task, which D55 forbids. Keeping the task id in orch-store, where a human can't read or edit it.
**Tag.** [default] · remind-me: pending
**Key.** lel:599:maintenance-task-registry
**Source.** let-em-loose-expert, 2026-11-04, answering the #599 build thread, from D55, D58 and 599-build-plan.md §4.

## D81. #599 lesson trigger 2 detects reverts from the action mark and git

**Decision.** For "accepted claim reverted within 14 days", the merge receipt is the claim's action mark: `claim_acting(claim, epoch, ref)` followed by `claim_action_result(done)`. orch resolves the mark's `ref` to the merge commit and looks for a commit whose message reads `This reverts commit <sha>` for it. It times the gap from the two commits' git committer dates, and the lesson fires when the gap is 14 days or less. A manual undo with no revert commit is not detected.
**Why.** The log has no wall time. C-R9 already records the action and names it through `ref`, so no log change is needed (D58). A missed lesson costs little, because lessons only inform (weight 2, never decide).
**Rejected.** A new merge-revision field (a log change). Treating a later reopen or drop as a revert (that is already the claim's own history, not a revert).
**Tag.** [default] · remind-me: pending
**Key.** lel:599:revert-trigger
**Source.** let-em-loose-expert, 2026-11-04, answering the #599 build thread (plan §3, trigger 2 [filled]), from addendum § Action marks and C-R9, and the card #599 design comment.

## D82. #704 slice 2: erosion profile recognition, invocation, opt-out, base, measured set and modes

**Decision.** Build detail for the `erosion` bin and host profile (card #704 Build 2), inside D41 and D42:
- `open_claims_task` recognises an erosion profile by its profile name, `erosion`. The recorded opt-out is an optional `ProfileSet` field `erosion` that accepts only `"off"`. It defaults when absent and isn't written then, so existing profile digests don't change, and it is covered by the digest when present. The open refuses with `bad_claims_config(erosion)` when a claims set has neither the profile nor the opt-out, has both, or has another value. The refusal applies from this slice; callers and tests add the profile or the opt-out in the same PR.
- The compare base is an explicit `--base <full commit id>` in the frozen args. The open checks that it is a full commit id and, when `ProfileSet.baseline` is set, that the two are equal.
- Invocation contract (amended after review on PR #1494): the `erosion` profile runs the bin directly. `program`'s basename is `erosion` (any path), `args[0]` is `check`, and the remaining args parse exactly as the bin's own parser does: `--base <commit>` once, plus either `--report-only` or both `--max-concentration-bp N` and `--max-duplication-bp N`, each flag at most once, with no positionals and no unknown flags. No wrappers in this slice: a shell (`sh -c`), `env` or `cargo run` form is refused with `bad_claims_config(erosion)` and a message naming the direct form. Recognition stays by profile name, so a profile named `erosion` with any other program is refused, not skipped. A wrapper can be added later with its own parser if a real caller needs one.
- The measured set is the whole tree at both commits, with exact `Ratio` deltas and the bp value beside them.
- Modes are frozen args: `--report-only` (always exits 0), or enforcing with both `--max-concentration-bp N` and `--max-duplication-bp N`. The two are mutually exclusive. Changing the mode changes the digest, so it only affects newly opened tasks (D42). The bin always prints one JSON line to stdout (the run log) with the per-metric deltas, the three worst-affected functions and the `unmeasured` list, for the inbox (Build 3, D43) to parse.
- D42's pause-at-expiry is not in slice 2. It is its own later slice and must merge before the profile goes live, because the time box starts at go-live.
**Why.** These are the most reversible choices that fit the approved design. Profiles are already identified by name in the run log and the fail-block; recognising by name and then refusing any program but the bin means a misconfigured profile is refused rather than silently skipped. The host can only vouch for `--base` and the mode if it parses the exact argv the bin will run: a reviewer showed that `/bin/sh -c "erosion check --base B …" --base A` passed a standalone-pair scan while sh ran against B. Allowing no wrappers is the narrowest contract and is relaxed later without changing any digest. The opt-out sitting under the digest is part of D41, so it isn't a BBP schema change. Duplication against untouched code needs the whole tree, and calibration (Q9) uses the same measure, so the limits absorb the scale. QA.md § Erosion gate puts the thresholds in profile args. `ProfileSet.baseline` is the pinned base (#597 G2, and the base that #598's `bbp open --base` stores; D62).
**Rejected.** Recognising the profile by the program's basename alone (a misnamed or wrapped profile would be silently skipped). Accepting wrappers such as `sh -c`, `env` or `cargo run` in this slice (the host would have to parse shell source or the wrapper's argv to trust `--base`; superseded the first version of this entry, which said wrappers still count). Measuring only the files the diff touches (it misses duplicates of untouched code). An unchecked free `--base` arg. Building the expiry pause into slice 2.
**Tag.** [default] · remind-me: mem_cc7bf0c6331140d8816112c351b44ed6
**Key.** lel:704:erosion-profile-build-defaults
**Source.** Questions Q1–Q6 from the #704 slice 2 build thread, 2026-11-04; D41, D42, D43, D62; QA.md § Erosion gate; card #704 Build 2 and Time box; `rusty_bbp_host` `src/profiles.rs` `ProfileSet`, `src/admin.rs` `open_claims_task`. Numbered D82 because open PR branches already use D78–D81. Invocation contract amended 2026-11-04 from the #704 slice 2 build thread, answering dottiusmaximus's review of PR #1494; `rusty_bbp_host` `src/profiles.rs` `check_erosion_base`.

## D83. #599 watcher dedups by finding key and slot in `watch_finding`

**Decision.** `rusty_orch watch` gives each finding a stable key and a slot: an advisory is keyed `advisory:<id>:<crate>` and a yanked crate `yanked:<crate>:<version>`, each its own slot; the update set is keyed `updates:<16-hex digest of the sorted set>` in slot `updates`; the toolchain is keyed `toolchain:rust:<watched>` in slot `toolchain:rust`, and the app's channel is read as TOML from `[toolchain] channel` (a basic or literal string, with an optional trailing comment on the header or the value; a bare or escaped value, or a key outside `[toolchain]`, isn't read). D58's orch-store `watch_finding` table (`watch.mmap`, `WatchRecord::v1`, keyed by app and slot, under the new required `--state DIR`) holds each slot as a `WatchSlot`: `posted` (the key last posted and its claim) and `pending` (a key and the exact post arguments as JSON). If the key matches the row, nothing is posted and the report gives the existing claim (`new: false`). A new key in an occupied slot is posted with `supersedes: <row's claim>` and the row is updated, so accepting the new claim retires the stale one (addendum `supersedes`, CL-53). An empty slot is posted, then the row is written. Details that can change (an advisory's title and cvss, the app's current pin) go in the report's `detail` and are never posted. Crash safety comes from a write-ahead pending post plus R15 replay. Before a post is sent it is saved as the slot's `pending`; once BBP answers, the claim replaces it in `posted`. If saving the pending post fails, nothing is sent. The next run resends a slot's pending post before anything else in that slot: if it landed, BBP replays it (same op, same bytes), otherwise it is posted now, and either way its claim becomes the slot's, so the next finding supersedes it. If BBP checks the op and refuses the resend, the post never landed, so it is dropped and the run reports one error. A refusal BBP returns before its op lookup (`forbidden`, `not_assigned`, `wrong_mode`, `bad_credential`, `op_conflict`, or a host error that isn't a BBP rejection) says nothing about an earlier send, so the pending post is kept and the slot waits for a later run. The op id is `watch-` + sha256(app + "\n" + key + "\n" + the row's prior claim id, empty when the slot is empty)[..32], so a post is identified by app, key and the slot's previous claim. Every posted field (text, domain, low stakes, reversible, `refs` = `art:1`, `supersedes`) is a function of the key and that prior claim, so the resent bytes match the first send exactly. A key that returns (update set A, then B, then A again, or a watched-Rust rollback) is a new op: a new claim that supersedes B, not a conflict with A's first post. Known limit: an advisory that is fixed and later returns keeps its old row, so nothing new is posted even if that claim was settled; the report still lists it with its old claim id.
**Why.** R15 replays only a byte-identical payload (`rusty_bbp/src/engine.rs` `replay`), so a key-only payload makes repeats safe. But a new update set or a new watched Rust is a new key, and with replay alone the earlier open claim would stay beside the new one. The slot row lets the new claim supersede it, which is what Nano asked for: an update or a supersede, not a second post. The prior claim is in the op id because, with app and key alone, a returning key A reused A's first op with `supersedes = B`, a changed payload that R15 refuses with `OpConflict` on every later run. The pending post is needed because R15 only helps when the same op is sent again. If A is recorded, B posts but its table write is lost, and the next run sees C, then C supersedes A and B is left open with nothing pointing at it.
**Rejected.** R15 replay alone, without the table: rolling slots would leave old open claims beside new ones. The affected range or the current pin in the key: a detail edit would post a duplicate. The current brief in `refs`: a brief revision would turn repeats into `OpConflict`. An op id from app and key alone: a returning key conflicts with its first post. Relying on R15 alone after a lost table write: when the next key differs, the lost post B is orphaned.
**Tag.** [default] · remind-me: mem_c2072da6e00e4410a81be5011b9fa732
**Key.** lel:599:watch-dedup-key
**Source.** Nano asked for the dedup to be logged with its key and reason in thread message cmsg_014AqbZs58L565hMV2pFkCX56GR1HMdW1176q2egkeFntF (2026-11-04), and dottiusmaximus's review comment 6643815580 (P2) on #1491 asked for the payload fix; their review comment 6663660803 (P2) on #1491 found the returning-key `OpConflict`, fixed by adding the prior claim to the op id. The edited 6663660803 and Codex review comment 4483261550 on #1491 found the orphaned post after a lost table write, fixed by the pending post. Codex review comment 4483261567 on #1491 had the toolchain channel read as TOML. let-em-loose-expert, 2026-11-04, answering the #599 build thread, found the rollover gap. Built in `rusty_orch/src/watch.rs` and `orch-store/src/watch.rs`; tests `watch::tests::changed_details_keep_the_first_claim_and_post_nothing`, `a_newer_finding_in_a_slot_supersedes_the_open_claim`, `a_post_whose_record_was_lost_is_settled_before_the_slot_moves_on`, `a_post_that_was_never_written_down_is_never_sent`, `a_refused_post_leaves_the_slot_as_it_was`, `a_key_that_returns_supersedes_the_slots_claim_without_a_conflict`, `the_board_refuses_a_changed_payload_under_one_op`, `the_channel_is_read_as_toml`, `watch::e2e::findings_are_posted_once_and_superseded_through_a_real_bbp` (Orch BBP acceptance job `watch-replay`; it also covers the A→B→A return, and a lost table write followed by a different key across a session restart), and orch-store `watch::a_watched_finding_survives_reopening_and_is_per_app_and_slot`.

## D86. #598 build defaults while 598-gaps-design.md is unavailable

**Decision.** Until the design file can be read, #598 is built with these choices inside the approved §6 (D59, D62). `task_paused.reason` is one of `spend_ceiling`, `call_ceiling` or `breaker`, and `detail` is free text. `intent_revised.affects` is `{all: true}` or `{claims: [..], domains: [..]}` (their union); an unknown claim, or both lists empty, is refused `bad_affects`. `intent_version` is a fold: 1 at `task_opened`, plus one per `intent_revised`. A claim's proven-against version is the `intent_version` at its latest acceptance, also a fold, with no stored field. An intent reopen follows C-R8's epoch and budget rules, with no automatic extension, and an acted-on claim escalates `intent_changed` instead of reopening. `bbp open --base <ref>` resolves the ref to a full commit id stored as the existing `profiles.baseline`. `baseline_digest` is the SHA-256 of the canonical baseline report at `<dir>/baseline/<task>.json`. `baseline_red` lists the red profile and test names plus a suggested profile-set edit as text, with no new skip field. `base_test_changed` refuses a candidate that modifies or deletes a `test_paths` file existing at `base`, unless its claim is `reversible: false`; such a candidate goes to the inbox, one item per candidate. Breaker settings are `window_calls` 40, `window_usd` 5, `min_score_move` 4 (sum of absolute score change in the window) and `mode` `observe`. `rusty_orch lens breaker` is a read-only report. `max_class` keeps the merged default by provider kind (ADR-0021) and `price` stays required. The shadow-run module is `shadow_run.rs`, because `shadow.rs` is #596's policy log.
**Why.** These are implementation details inside an approved design. Each is the most reversible option consistent with card #598, D59/D62, QA.md § Gaps and the merged orch-lens code.
**Rejected.** Waiting for the file; adding new profile-set or claim fields beyond §6 as recorded.
**Tag.** [default] · remind-me: mem_6b6e898a0a3d4a4bb3d374fc8f9dae41
**Key.** lel:598:build-defaults-without-design-doc
**Source.** let-em-loose-expert, 2026-11-04, for the #598 build session; OPEN_QUESTIONS Q16.

## D88. #704 D42 expiry pause: go-live, count, what pauses, opt-out

**Decision.** Build detail for the D42 pause-at-expiry slice (card #704), inside D41, D42 and D82:
- Go-live and the count come from a host-side, append-only ledger, `<dir>/erosion/report-only.log`, one line `<unix_ms> <task id>` per report-only claims open. A claims open writes no time to the BBP log (`task_opened` has no time field and claims mode grants no turn at open), so the log alone can't date it. In `open_claims_task`, after every refusal check and before the open dispatches, the host appends the line; if the append fails, the open is refused and nothing is opened, so no report-only task can open uncounted. `admit` returns an `Admission` guard holding the ledger's exclusive lock, and `open_claims_task` keeps it until `open_mode` has published the task log, so a racing report-only open always sees the earlier one counted (two opens at a count of 9 can't both pass). Go-live is the first line's time. A ledger task id counts only when its own log starts with a claims `TaskOpened` whose frozen profile set, loaded and checked against `profile_digest`, runs the erosion profile report-only; if that log or profile set can't be read, the open fails closed. A line for a task that never opened, or opened with `"off"` or enforcing, starts the clock but never counts; starting it earlier is the safe direction under D42. A missing ledger means not live yet. This is host state, not a BBP change, and no human command writes it. Someone with write access to the data dir can reset it, but they can already rewrite the task logs.
- Only claims tasks with a report-only erosion profile count. Report-only expires 14 days after go-live or once 10 such tasks have opened, whichever comes first. After that, `open_claims_task` refuses a new claims open with a report-only profile with `bad_claims_config(erosion)`: report-only has expired, and limits wait for Nano's approval from the calibration numbers (card #704 Build 5). Tasks already open are untouched.
- An `erosion = "off"` opt-out is not an affected task: it doesn't count and isn't paused.
- Count by tasks opened, not tasks finished.
- Sending the calibration numbers to Nano is Build 5 (card #704 Build 5 and Done when), not this slice. The refusal only points at it.
- Enforcing opens are unchanged for now. Whether an enforcing profile's limits need a human-only host record of the approved limits is OPEN_QUESTIONS Q20.
**Why.** These are the most reversible choices that fit D42. The ledger dates every report-only open with no BBP schema change. Appending before the open fails closed, so the time box can't be undercounted. The pause acts on opens, so counting opens keeps the time box from stretching while tasks run. The opt-out is D41's recorded path, covered by the digest, and pausing it would pause every claims task, which D42 doesn't say.
**Rejected.** A go-live file written by a human command (the clock could be started late). Deriving go-live statelessly from the first `TurnGranted` (an unassigned task would never start the clock, extending report-only against D42). Appending after the open (an open whose append failed would stand uncounted). Counting any ledger id that has a task in the store (a task log doesn't prove a report-only open). Releasing the ledger lock before the task log is published (two racing opens could both pass at a count of 9). The first version of this entry, which read the open time from an opening `TurnGranted` that claims mode never writes. Counting finished tasks. Pausing opt-out tasks. Delivering the calibration report from this slice.
**Tag.** [default] · remind-me: mem_b55d930642a747baab2f93387bc09ad3
**Key.** lel:704:expiry-pause-build-defaults
**Source.** Questions Q1–Q6 from the #704 expiry-pause build thread, 2026-11-05; D41, D42, D82; QA.md § Erosion gate; card #704 Build 5, Time box and Done when; `rusty_bbp_host` `src/admin.rs` `open_claims_task`, `src/profiles.rs` `check_erosion`; `rusty_bbp` `src/event.rs` `TaskOpened`. Counting rule and lock span amended 2026-11-05 after dottiusmaximus's review of PR #1508 (builder default, not a Nano decision). Open-time bullet amended 2026-11-05 after the coordinator's probe of `admin::open_claims_task` on main showed the open writes only `TaskOpened` and `ArtifactStored(Brief)`. Numbered D88 because open PR branches already use D78–D87.

## D91. #704 slice 3: erosion escalation detail in the inbox

**Decision.** Build detail for card #704 Build 3, inside D43 and D45:
- There is no erosion escalation reason. An erosion escalation is any open `bbp inbox` escalation whose claim has `erosion` in `failing`. Its detail comes from that `FailingView`'s report, then the report's log, then the `{"erosion":1,...}` line in the log's `=== erosion :` section.
- `unmeasured` code is shown on the claim whether or not the profile fails (D45). For an escalated claim without a failing erosion profile, the host reads the latest report on the claim's current candidate that has an `erosion` profile result.
- `Escalation` gains an optional `erosion: Option<ErosionDetail>`. It is additive and is not a BBP schema change. The typed fields mirror the bin's line: `base`, `mode`, `within_limits`, `concentration_bp` and `duplication_bp` as `{base, candidate, delta}`, `total_cc`, `loc`, `functions`, `worst_affected` (at most 3 `{key, base_cc, cc}`), `unmeasured` and `parse_errors`. Two more fields come from outside the line: `failing: bool`, taken from the profile result, and `error: Option<String>` for the bin's error line. If the log can't be read, has no erosion section, has no parsable line, or reports a version other than 1, the field is `None` and the inbox still answers.
- The line is parsed on the host side with serde in `rusty_bbp_host`, so the host doesn't depend on `rusty_erosion` and tree-sitter. Both crates' tests pin one golden line taken from the bin's `render`.
- Rendering in the Mill Board inbox (#595) is part of Build 3, because "Done when" isn't met by JSON alone. It may land as a second PR after the host JSON. The board shows the deltas and worst functions when erosion is failing, and shows `unmeasured` always. Whether passing or report-only deltas also show on other escalations is OPEN_QUESTIONS Q23. Until it is answered, they are in the JSON but not rendered.
**Why.** No `ClaimEscalation` variant exists for erosion, because a failing profile blocks through the fail-block (D30, D41). The failing profile on the claim is therefore what makes an escalation an erosion one. D45 puts `unmeasured` on the claim and keeps it from escalating. An optional, typed, fail-soft field is the most reversible shape. A host-side parse keeps a parser dependency out of the host, and the shared golden line catches drift. The board is the inbox Nano reads.
**Rejected.** A `pub fn parse` in `rusty_erosion` used by the host, which pulls tree-sitter into the host. A JSON-only slice 3 that closes Build 3 without a board view.
**Tag.** [default] · remind-me: mem_136ada292af5416ebec3ecaf53b3dad9
**Key.** lel:704:escalation-detail-build-defaults
**Source.** Questions Q1–Q4 from the #704 slice 3 build thread, 2026-11-05; D30, D41, D43, D45, D82; QA.md § Erosion gate; card #704 Build 3 and Done when; `rusty_bbp_host` `src/inbox.rs` `Escalation`, `Reason`; `rusty_erosion` `src/main.rs` `render`. Numbered D91 because open PR branches already use D89–D90. Its open question is Q23 because open #1520 uses Q21–Q22.

## D92. #599 track records: job rows, pivotal scope, ground truth, weight, PR split

**Decision.** Build detail for #599 track records (599-build-plan.md §2), inside D58 and D93:
- **Job rows.** The D58 orch-store table `lens_record` holds one row per answered blind `Review` or `LookHarder` job: task, lens, claim, epoch, slot, the MsgIds the job posted, and an `unsure` flag. A job answered with nothing, or whose reply the protocol refused, is a row with no items and counts 0 in the denominator, so refusing isn't free. `Skipped` jobs and jobs no provider could answer are not written. A row is saved before the scheduler state. Saving the same job key again unions its items, so a job rerun after a crash keeps every item it posted, and leave-one-out removes them all together.
  - **Partial posts.** When a post fails partway (`PostError { posted, unsure, error }`) after something landed, or after an answer was lost, the job is recorded as answered with the posts that landed. Its row keeps their ids, the scheduler marks it answered over that prefix, and it is never asked again. That way no fresh evidence is posted next to what is already on the board. An answer counts as lost when the one same-op resend didn't return it, which means the post may have landed under an unknown id. The row is then marked `unsure`, and the flag stays set when the row is saved again. A job with nothing landed and no lost answer is abandoned and asked again as before.
    - **Remaining window:** a crash after the row is saved but before the scheduler state is kept. The job can then be asked again from the older state. Its new items are added to the same row, the union rule above, and a sticky `unsure` keeps it unscored.
  - **Write-ahead.** Before a job's first post, its row is saved with `pending` set and no items. So is every other row an attempt saves, an empty answer's or a refused reply's included, so a save that clears `pending` always completes its own attempt. The post-round save clears `pending` and adds the job's items to the row. If a job's write-ahead finds its row still `pending`, an earlier attempt crashed or failed its save and may have posted under ids nobody knows, so the row is marked `unsure` and the flag stays set. A row still `pending` when scored is treated like an unsure job: never scored, pivotal on a line settlement, and kept in `assigned`. A crash before any post leaves a pending row with nothing on the board, which is unscored too. That is conservative: it costs a point the lens may have earned, and it never awards one. `pending` is part of the v1 row with a serde default, because the table hasn't shipped.
  - **Unsure jobs.** An unsure job is never scored. Its lost item could change all of these:
    - its direction, since the item may go the other way;
    - whether it cites the golden report, since the item may be `executed`;
    - its weight, since `upset` is judged from its first own item.

    So scoring its known items, even against golden or a `Human` or `Choose` outcome, could award points the record doesn't support. On a line settlement it counts as pivotal, and it is not pivotal on a `Human` or `Choose` settlement. Either way it stays in `assigned` and is never in `scored`. No id is inferred, and nothing is replayed under a new op.
  - **Other details.** `unsure` is part of the v1 row, because the table hasn't shipped. Points are folded from the rows and `claim_history` when read, never stored. `Record`'s `found_problems`, `found_nothing_twice` and `refused` fields are left as they are, not wired and not deleted, in this PR.
- **Pivotal scope.** A claim's settlement is its latest `Status` step that changed its status. A step that leaves the status unchanged, such as an `ActionUnconfirmed` escalation that leaves it `Accepted`, settles nothing. If that settlement isn't `Accepted` or `Dropped`, the job is pending. Leave-one-out applies only when the settlement is `AcceptLine` or `DropLine`. A `Human` or `Choose` settlement didn't depend on the score, so the job is never pivotal and is scored against that outcome. That is how a later Nano reversal acts as ground truth: no separate reversal source is needed, and golden and reversal can't conflict.
- **Ground truth for a pivotal job.** The final `golden:` report (not `holdout:`, which a model wrote) on `current(c)`'s code at settlement: passed means accept was right, failed means drop was right. It doesn't count for a job whose own items cite that report. There is no shadow-run source yet; #598's approved `shadow:` profile kind (D62) is the expected producer. Ground truth is an internal function in `orch-lens::record`, with no new port.
- **Weight.** A pivotal job scored against ground truth uses the same `w = 1 + contested + 2·upset`, with its own items removed and `upset` judged against the ground truth.
- **PR split.** Track records are a PR of their own; lessons (§3) and `lens why` (§5) follow in later PRs.
- **Interim limits (2026-11-05, ended by D112 in PR #1534).** Until D112, `claim_history` didn't carry everything the rules above need, so the first build was conservative. Each limit only makes more jobs unscored-pivotal. None scores a job against the board outcome, and all of them show in the `pivotal` count. The code and 599-build-plan.md §2 note them.
  1. *Author minimum.* A line rule doesn't carry `min_authors`. If removing the job's items would remove the lens as a distinct author, the job counts as pivotal.
  2. *Woken items.* The history keeps a woken item's append-time epoch and `scoring` flag, and has no `dormant` flag. An `Evidence(m)` reopen counts item m as live in the new epoch. `Dep` and `Intent` reopens wake nothing. A `Human` reopen wakes dormant items. BBP marks an item dormant only when it is `executed` evidence posted against a closed claim's status (counter while accepted, support while dropped) while a dep was closed, and the history doesn't show the dep. So if a `Human` reopen into the settlement's epoch exists and any item on the claim, from any author, fits the kind, direction and status test, every job on that line settlement counts as pivotal.
  3. *Golden provenance.* Only a `Failing` step shows a `golden:` result, so a passed golden set isn't visible. BBP keeps `failing` when `current` changes, and the history has no candidate steps, so a failing step can't be tied to the candidate current at settlement. The pure rule gives golden ground truth (drop was right) only when the failing report tested the candidate current at settlement. The `bbp why` path carries neither, so golden truth is withheld there until Q35. D93's required test runs on the pure rule.
  4. *Citation.* Evidence steps carry no refs, so a job whose own items include `executed` evidence gets no golden ground truth. This stands in for "its items cite that report".
  5. *Ended.* D112 (PR #1534) ends all four limits:
     - Line rules carry `min_authors`.
     - Reopen steps name the items they woke, and only those count in the new epoch.
     - Golden truth comes only from the holdout report that the status step of a line settlement names (`holdout`, BBP's `holdout_truth` on the state just after the change), on the `bbp why` path as well. That report is the latest non-error final report from a holdout candidate building the current code, under the task's holdout digest at that moment, on the tree the current candidate's latest report tested at that moment. So a later suite update or a moved tested tree can't leave an older report standing in. Any `golden:` profile failed means drop was right; any passed with none failed means accept was right; a named report with no `golden:` profile, or no named report, gives no truth. A runner-error report (`RunStatus::Error`) never becomes a `Report` step and is never truth.
     - A job whose items cite that report's id gets no truth from it, which replaces the `executed` stand-in.

     A history from an older bbp without `min_authors` is refused (`msg:<claim>: line has no min_authors`) rather than read under these limits. The refusal stays inside the record:
     - `lens run` logs `pass N track record not read: <error>` and keeps going, and each lens's context leaves its `record:` line out.
     - `lens record` exits 1 with the same message.
**Why.** These are implementation details inside the approved design. Each is the most reversible option: scores are computed when read, so a later rule change recomputes them from the stored rows. Only `golden:` sets are signed off by Nano, and a holdout lens can't be judged by its own report. A port would add to D58's approved list, which the build plan says nothing does.
**Rejected.** Storing points. Leave-one-out on human settlements. Treating a reversal as a separate source, ranked against golden. `holdout:` reports as ground truth. A `GroundTruth` port now. One PR for track records, lessons and `lens why`. Inferring an unknown post's id, or replaying it under a new op. Scoring an unsure job's known items against ground truth. Leaving the post-before-save window as an interim limit. Letting a retry's save clear an earlier attempt's `pending` without marking the row `unsure`, including a retry that posts nothing (an empty answer or a refusal).
**Tag.** [default] · remind-me: mem_6f0738d7778847d0b13fac1f7d458b68
**Key.** lel:599:track-record-build-defaults
**Source.** let-em-loose-expert, 2026-11-05, answering questions 1–4 and 6 from the #599 track-record build thread; D58, D93 (PR #1518), 599-build-plan.md §2, `bbp-claims-addendum.md` § Holdout suites (G1), `rusty_bbp::history::Rule`. Question 5 (a `lens record` subcommand) is OPEN_QUESTIONS Q34. Interim limits: coordinator follow-ups A and B, 2026-11-05; `claims_engine.rs` `accept_line`/`drop_line`; `rusty_bbp::history` `Change`, `Rule`; OPEN_QUESTIONS Q35. PR #1520 Codex findings on reopen activation and golden provenance, 2026-11-05; `claims.rs` item append (`dormant`), `ClaimReopened` fold, `on_artifact`. PR #1520 review fixes on the settlement step and partial posts, 2026-11-05 (commits 89490a8f1, 619b735c5); `Board::post` `PostError`. dottiusmaximus's re-review question on PR #1520, 2026-11-05 (commit 5fcf62541). PR #1520 Codex finding on duplicate board effects, 2026-11-05 (commit 570e88396, `round::partial`). PR #1520 Codex finding on posts lost before the row save, 2026-11-05 (run.rs:295-299, discussion_r4485013131); dottiusmaximus's follow-up on retries that post nothing, 2026-11-05 (comment 6667319872).

## D93. #599 track records score pivotal reviews against ground truth only

**Decision.** Option A plus ground truth. A pivotal review is one whose outcome flips once the lens's own evidence is removed. It is not scored against the board outcome, and each lens reports a pivotal count beside its points. Where the claim has ground truth (a golden-set test, a shadow-run result, a later Nano reversal), the pivotal review is scored against that instead. With no ground truth it is unscored and counted as pivotal. The 20-item calibration report shows, per lens: points, scored count, pivotal count, and pivotal-with-ground-truth count. Required tests: pivotal with no ground truth is unscored and counted; pivotal with ground truth is scored against the ground truth; a non-pivotal review is scored as before; a lens that flips an outcome it bet on earns nothing from the flip. This answers OPEN_QUESTIONS Q17.
**Why.** Ground truth isn't moved by the lens's vote, so scoring against it can't be gamed. Reporting the counts keeps "unmeasured" visible, so it doesn't read as "bad".
**Rejected.** Scoring pivotal reviews against the board outcome, which the lens itself moved. Leaving every pivotal review unscored, even where ground truth exists. The other Q17 options: pivotal reviews at w = 1, or dropping the pivotal rule.
**Tag.** [stated] · remind-me: mem_3f1158fb8fe0485697a8a713fc86469f
**Key.** lel:599:track-record-pivotal
**Source.** Nano, thread message cmsg_014AqbZs58L565hMV2pFkCX56GR1HMdW1176q2egkeFntF (2026-11-04), "Track-record question: option A, plus ground truth". OPEN_QUESTIONS Q17 (card #599 Q2). Numbered D93 (first drafted as D84) under the 2026-11-05 allocation that gives this thread's open PRs D92–D95. Confirmed by Nano's tap of option A, "Keep, report a pivotal count", on decision card cmsg_014AqbZs58L565hMV2pFkCX5FmbGRzqBvub8s2BWcnKhkV (thread cmsg_014AqbZs58L565hMV2pFkCX5Fiyuxpido5spNwMT4HHhuz), 2026-11-05T21:15:17Z (tap message cmsg_014AqbZs58L565hMV2pFkCX5E4QqeAo7EVN379KMrdxNJa). The card was posted before the 11-04 message and doesn't mention ground truth. The ground-truth scoring was confirmed when Nano told dottiusmaximus directly "1518 and 1520 I approve", after the queue had explained D93's pivotal-review ground-truth scoring. dottiusmaximus recorded this on PR #1518 (comment 6669751039, 2026-11-05T21:15:38Z) and PR #1520 (comment 6669752737, 2026-11-05T21:15:43Z). Neither comment claims the 11-04 thread message was retrieved independently.

## D94. Let Em Loose designs live in the repo, in their first draft PR

**Decision.** Every Let Em Loose design goes into `docs/projects/let-em-loose/` as part of its first draft PR, never only in `/mnt/project-files/` or session scratch. The rule is in the let-em-loose-expert pack (sources, step 4) and the let-em-loose row of the scoped `CLAUDE.md` rule.
**Why.** `/mnt/project-files/` doesn't survive between sessions: the #594 and #599 designs were both lost there.
**Rejected.** Keeping designs in `/mnt/project-files/let-em-loose/` or in session scratch.
**Tag.** [stated] · remind-me: mem_aff6060cd75743ef9807b92006c3f1c7
**Key.** lel:general:designs-live-in-repo
**Source.** Nano, thread message cmsg_014AqbZs58L565hMV2pFkCX56GR1HMdW1176q2egkeFntF (2026-11-04), "Design docs live in the repo, never in session scratch." Numbered D94 (first drafted as D85) under the same allocation. Confirmed when Nano told dottiusmaximus directly "1518 and 1520 I approve", after the queue had explained D94's designs-in-repo rule. dottiusmaximus recorded this on PR #1518 (comment 6669751039, 2026-11-05T21:15:38Z), which doesn't claim the 11-04 thread message was retrieved independently.

## D96. #708 orch runs the alien lens's probes itself, sandboxed

**Decision.** orch runs the alien lens's model-written probe tests itself, in a sandbox. A `SandboxProbes` adapter in `rusty_orch`, behind orch-lens's `Probes` trait, builds a scratch worktree of base plus the candidate's diffs. It runs `cargo test -p <crate> --test probe_<claim>_<n>` with `--no-run` first, then runs the test on the candidate and on base, all with no network. `rusty_orch` adds the in-workspace crates `rusty_sandbox` and `rusty_worktree` as dependencies (they are already workspace members and `[workspace.dependencies]`, so no external crate or version changes). This was on the card as option A's stated consequence, so Nano's choice covers it. D75 §6 assumed an "existing lens sandbox"; orch had none, and both crates were used only by `rusty_bbp_host` (`run_control.rs`, `confine.rs`). This entry replaces that assumption. v1 stays shadow-only, and the trait leaves room for v2 to move execution to the host.
**Why.** It keeps v1 shadow-only with no BBP change, and it reuses the host's crates and no-network confinement. Without probe runs, the §8 success bar can't be measured. The cost is a code-running path in orch. D20 still holds: the lens runner already extends orch, so this adds no fourth executor.
**Rejected.** (b) Running probes through the BBP host's run machinery: that is the deferred v2 BBP route and needs its own design. (c) Holding probe execution, which would ship hunts and probe generation with no runs and leave §8 unmeasurable.
**Tag.** [stated] · remind-me: mem_936fff49c7ec468fb8da5434da55ed73
**Key.** lel:708:probe-execution-in-orch
**Source.** Nano tapped option A, "Yes, orch runs them sandboxed", on hearthbot decision card cmsg_014AqbZs58L565hMV2pFkCX53jwAvbS5K1FLg3DH1ESfus (asked 2026-11-04T23:52:18Z, answered 2026-11-05T02:31:46Z), thread cmsg_014AqbZs58L565hMV2pFkCX5PRArQZK27dSqRZa2ZYPeC1. Option A's consequence read: "orch adds rusty_sandbox and rusty_worktree and runs probes with no network on a scratch base+diff tree, behind a trait so v2 can move it to the host." `/mnt/project-files/let-em-loose-708/708-slice4-plan.md` § Findings, 4d. Drafted as D89 and renumbered D96 on 2026-11-05 under the coordinator's allocation (D96–D99 for this thread), because main and open PR branches use D78–D95; meaning, key and tags unchanged.

## D97. #708 slice 4d: probe scratch trees and cargo time limit

**Decision.** Build detail inside D75 and D96.
1. **Scratch trees.** Trees go under the lens state dir (`--state`) at `<state>/alien/probes/<task>/<claim>-<n>/`. There is one tree for the candidate (base, the candidate's diffs and the probe file) and one for base (base and the probe file). A guard removes both after that probe's runs, whatever the result, and the runner removes any leftover `<state>/alien/probes/` when it starts. The candidate's cargo target dir, `<state>/alien/target/<task>/<claim>`, persists between that hunt's probes as a build cache, is emptied when a hunt starts, and can be deleted at any time.
2. **Time limit.** Each cargo invocation, the `--no-run` build and then the test run, has its own wall-clock limit: the named const `PROBE_CARGO_LIMIT`, 10 minutes. Cargo runs `--offline` inside the no-network sandbox. A timeout on the candidate, in the build or the run, is reported as `Run::BuildError`, so its probe row is `error` with a "timed out" note and never counts as a finding. A timeout on base is returned as a `Probes::run` `Err`: no probe row and a "not run" note, never `new-code-fail`. Failures of the worktree or sandbox setup are `Err` too. The orch-lens `Run` and `Outcome` types do not change. A base build killed by a signal is handled like a base timeout.
3. **Isolation.** Each tree gets its own new temp dir (`<name>-tmp`, also its `HOME`) holding a new private cargo home. Base builds into a target dir inside its own temp dir, never the candidate's cache, so nothing a candidate run wrote reaches base. Host-side writes into a tree never go through a symlink, because the candidate's diffs can add them. Every directory from the tree root down to the crate's `tests` must be a real directory, and the probe file is created new (`O_EXCL`). A site path under a symlinked directory names no crate. A refused placement is an `Err`, so the probe is not run.
**Why.** D75 §6 states no time limit and no location, so this is implementation detail. One const, the existing types and state that can be removed are the most reversible choices. Only a probe that passes on base and fails on the candidate is a regression. A hang therefore can't be counted as one, and a base timeout can't separate pre-existing from regression from new-code-fail.
**Rejected.** A system temp dir, which leaves trees outside the state the operator manages. Mapping a candidate run timeout to `Failed`, which could turn a hang into a false regression. Mapping a base timeout to `BuildError`, which would record `new-code-fail` wrongly. A new `Run::TimedOut` variant, an orch-lens public API change that isn't needed yet.
**Tag.** [default] · remind-me: mem_f7977776fd4044d081d68c7525aa514b
**Key.** lel:708:sandbox-probes-limits
**Source.** The #708 slice 4d build questions, 2026-11-05. D75 and design §6. D96. `orch-lens` `src/alien.rs` `outcome`. PR #1500 `src/hunt.rs` `Probes::run` (where `Err` means the probe did not run). Slice 4c rule: unrun probes are never written as `error`. Item 1's cache scope, the signaled-base rule in item 2 and item 3 were amended 2026-11-05 after review of PR #1511 by dottiusmaximus and Codex (builder default, not a Nano decision). The shared cache was narrowed because candidate code can write to it. Drafted as D90 and renumbered D97 on 2026-11-05 with D96, for the same collision; meaning, key and tags unchanged.

## D98. #708 alien hunt reads the candidate and its diffs through `get_artifact`

**Decision.** In slice 4b the lens runner reads the claim's `current` candidate, then each of its diffs, with `get_artifact`. Each artifact's first read is charged once to the lens principal under R11, against the task's `reads` total and not the claim's evidence budget. That fits §3, whose "nothing is debited" is about the claim's budget (addendum § Budgets keeps the two apart). Build detail:
1. If the role lacks `Read(Candidate)` or `Read(Diff)` (`role_forbidden`, which is rejected before any charge), the claim is not eligible. The runner prints the refusal once per run on stderr, writes no hunt marker (D78) and does not fail the run. Every other `get_artifact` error goes through the runner's usual error path.
2. Whether the candidate touches a `.rs` file comes from the diff text of all its diffs: any `--- a/`, `+++ b/`, `rename from`, `rename to` or `diff --git` path that ends in `.rs`, with `/dev/null` ignored.
3. If the combined diff text is longer than the enumerate prompt's window (a named const in `orch_lens::alien`), the hunt is skipped, not truncated. Nothing is spent on a model call, and no probe row and no hunt marker are written. The runner prints one stderr line per claim per run. Whether to hunt oversized diffs file by file is open as OPEN_QUESTIONS Q18.
**Why.** These are implementation choices inside the approved D75 design. Each one is the most reversible option. A refusal or a skip never escalates the task or fails a run, and it records no hunt, so a later fix makes it retry. A truncated diff would make the suspects unreliable. Diff artifacts carry text, not a file list.
**Rejected.** Failing the run when a role is refused. Truncating oversized diffs. Recording a skip as a hunt.
**Tag.** [default] · remind-me: mem_17ea768d43f24f95944d46d7abb9836d
**Key.** lel:708:alien-candidate-reads
**Source.** let-em-loose-expert, 2026-11-05, answering the #708 build thread's slice 4b questions. `708-alien-lens-design.md` §2–§3, D75, D78; `claims_engine.rs` `get_artifact` (R11 per principal and artifact); `engine.rs` `get_artifact_record`; `bbp-claims-addendum.md` § Budgets and § Reads, delta and subscriptions. Drafted as D87 and renumbered D98 on 2026-11-05 to avoid colliding with #1496; meaning, key and tags unchanged.

## D99. #708 slice 4c: `lens run --alien`, its config entry and its trigger

**Decision.** Build detail inside D75, D78, D96 and D98.
1. **Config.** The lens config takes an optional `"alien": {"id", "prompt", "providers"}`. Its id is its own BBP role and may not be an ordinary lens's id. Its providers must be CLI ones, because its calls use read-only tools (design §3). It follows every domain and has no `may_post`, so it posts nothing. Its prefix is intent, rules and its prompt, with no post protocol (`context::raw_prefix`).
2. **When.** `lens run --alien` (off by default) runs one alien pass after the ordinary passes, and only when they end quiet or at `--max-passes`. It does not run after a halted round or a task ceiling. Before any pass, it checks the `alien` entry, the task id (one path component), the toolchain and the `--repo` repository, opens the lens's session and removes leftover `<state>/alien/probes/` (D97), so a missing piece costs nothing.
3. **Trigger.** Until #596's value score exists, only irreversible claims are hunted, and there is no control sample (D75 Q1). "Every review lens has recorded its blind outcome and owed look-harder" (§2) means this: each lens that may post evidence and follows the claim has a blind pass in the claim's current epoch, and no look-harder is still owed under the scheduler's own rule (`schedule::reviews_settled`). A claim that no review lens follows is not settled. The cheap checks come first, then the candidate is read (D98).
4. **Skips.** If no alien provider is cleared for the claim's class, the base is not a full lowercase commit id (it reaches `git worktree add`), the probes cannot be set up (item 6), or no provider takes the enumerate call, the claim is skipped as D98 skips one: one stderr line, no row, and a retry next run. A ceiling before the enumerate call ends the pass. The probe rows are saved before the hunt row, which is the "hunted" marker (D78).
5. **Memory key.** "Already hunted" is keyed by task id, claim and candidate, as the meter is keyed by task id. So that a same-named task from another BBP directory cannot be mixed in, `--alien` binds each task in a state dir to one board (`<state>/alien/boards/<task>` holds `run::board`'s identity) and refuses another. Keying the rows by board would change the hunt row's schema; the binding needs no schema change. The state root is made absolute, since the sandbox refuses relative paths.
6. **Preflight.** Before the first call of a hunt, the candidate tree (base plus every diff) is built once and removed. A base missing from `--repo` or a diff that does not apply is then a skip (item 4), not a recorded hunt whose probes could never run.
**Why.** These are implementation choices inside the approved design, and each is the most reversible one. Ordinary lenses must have `may_post`, so the alien lens needs its own entry. A skip that records nothing can't hide a claim for good. Running after a halt would spend while the runner is failing.
**Rejected.** Making the alien lens an ordinary lens with an empty `may_post`, which config forbids and which would mix it into the scheduler (D78). Running the pass after a halt or a task ceiling. A new `HuntRow` key field. Building the candidate tree only inside the probe runs, which records a hunt whose probes could not run. Items 5 and 6 were amended 2026-11-05 after Codex review of PR #1524.
**Tag.** [default] · remind-me: mem_cbd770c24a464ce881d6173e02d28402
**Key.** lel:708:alien-run-pass
**Source.** The #708 slice 4c build, 2026-11-05 (builder default, not a Nano decision). `708-alien-lens-design.md` §2–§4, D75, D78, D96, D97, D98. `orch-lens` `src/config.rs`, `src/schedule.rs`. `rusty_orch` `src/lens/hunts.rs`, `src/lens/run.rs`. Last number in this thread's D96–D99 allocation.

## D100. Design mux-hosted agent ptys (D20 and D66's launch path reopened for that design)

**Decision.** Nano chose to design, not build, rusty_mux hosting the ptys of Let Em Loose agents (he asked whether the lens runner could be a mux-hosted agent). For that design only, D20 (no fourth executor) and D66's approved #600 builder launch path (the orch-bbp builder launch profile) are reopened. D25 still applies: the design goes on a card and Nano approves it before any code. This does not by itself decide that the mux becomes an executor or that process lifecycle leaves orch and `bbp mod`; those are the design's questions, along with the collisions in the convergence brief (sandbox has no Unix sockets, `RUSTY_MUX_SOCKET` inheritance against D13, input lease against D8/D13, mux status not being evidence under D3/D22). Until a design is approved, D20 and D66 stand as written.
**Why.** Nano wants agents visible and hosted in mux panes. Designing first keeps the choice reversible.
**Rejected.** A watch-only card after #600 (the recommended option A). A watch-only card now (option B).
**Tag.** [stated] · remind-me: mem_8d721e801bdf4d65a98e6035ad7c6b37
**Key.** lel:mux:hosted-agents-design
**Source.** Nano tapped option C "Design mux-hosted agents" ("The mux hosts agent ptys. This reopens D20 (no fourth executor) and the approved #600 design.") on decision card cmsg_01WgyiM31RsSnZTgNuPX2VE4MQRCC8E9wvaXezDJg8oqev ("Choose how agents should appear in rusty_mux panes"), 2026-11-04T22:28:51Z; preceded by thread message cmsg_01WgyiM31RsSnZTgNuPX2VE4Nuc7mwNdskgzVg6vK8gWfC (20:07Z), "could the lens runner not be a mux hosted agent?". Context: remind-me mem_d92731e7311c4e90ae9256036eff3cd1 (rusty_mux stays an agent substrate, no orchestration), PR #1470 roadmap arc 4. Numbered D78 on PR #1496's branch; moved to D100 on 2026-11-05 because main assigned D78–D88 first.
**Superseded 2026-11-04 by D103.** Nano decided that agents get mux-hosted ptys, so D100's "design only; doesn't decide that the mux hosts" framing no longer holds. Design first (D25) still applies.

## D101. Tools go through the gateway, and the agent process is jailed

**Decision.** #726's "every action passes through the sandbox via ACP" is replaced by two parts. First, tools go through the gateway: it extends agentgateway-mcp, not rp-mcp, and enforces per call on everything that leaves the jail (push, PR, merge, board, secrets, network). Second, the agent process is jailed, which limits what it can reach: namespaces and a cgroup, overlayfs mounts plus rusty_sandbox Landlock, egress denied by default except to the gateway, and seccomp. No secrets go inside; the agent gets only a short-lived token scoped to the lens and the claim. An agent_cloud microVM around it is optional, and required for untrusted agents. The agent's built-in tools stay on; edits inside the jail land only in the agent's own throwaway layer. Audit comes from a witness outside the jail, not from the agent: the upper-layer diff, an exec log (audit/eBPF by cgroup) and the gateway's tool-call log, posted on the claim as hashed digests. Denials and out-of-scope writes are posted right away. ACP `session/update` is self-reported and low-trust; a mismatch with the witness counts as evidence against the claim. ACP is the adapter orch uses to drive agent sessions (compared with the existing executors on #594), not a security boundary. Holes to plug: shared `.git` (a read-only object store, with a per-agent clone via `objects/info/alternates`) and shared sccache (read-only for critics, content-keyed writes for builders).
**Why.** Spike #727 showed ACP can't enforce anything: the Claude Code adapter stopped delegating fs and terminal in v0.18.0, Codex runs commands in its own sandbox, ACP v2 drops client fs and terminal, and `request_permission` is only a prompt. Risky actions all leave the jail through tools or the network, where the gateway and egress rules can govern them.
**Rejected.** Every action passing through the sandbox via ACP. Building the gateway on rp-mcp.
**Tag.** [stated] · remind-me: mem_b37d92b6e738462b95873adb4d4250ea
**Key.** lel:726:gateway-and-jailed-agent
**Source.** Mill Board card #726, comment by claude-cloud at 1793816610843 ms, "Decisions, Nano, 2026-11-04 (after spike #727)", D-a. Nano asked for it to be in this file in thread message cmsg_01WgyiM31RsSnZTgNuPX2VE42EMPL7EXicY3bTWURHCCkG (2026-11-04T23:51:22Z). Numbered D79 on PR #1496's branch; moved to D101 on 2026-11-05 because main assigned D78–D88 first.

## D102. orch and the gateway stay separate

**Decision.** orch decides: it composes the lens, picks the model, drives the ACP session and mints the token. The gateway enforces: it checks the token, applies the TOML lens policy, filters tools and passes identity upstream. The gateway runs as its own process, as a security boundary, while orch stays a monolith. Neither imports the other; their shared contract is the lens/policy schema plus the token's claims, kept in Factory config. The gateway also serves callers orch didn't launch. Upstream modes: stateless servers are pooled, with identity sent on each call (remind-me, Mill Board, Tick, web); GitHub is pooled per credential; stateful servers get a session per agent (sandbox fs/shell, browser, REA); unknown servers default to per-session. Gmail, Drive and Calendar aren't exposed. The gateway sets the actor on Mill Board and `written_by` on remind-me from the token, and strips any actor value the agent supplies. Build order (from #727): session tied to token (S), identity passed upstream (M), per-session upstream connections and teardown (L+M), TOML lens policy (M), sandbox fs/shell MCP (M), process jail (L), skill libraries (M).
**Why.** Separation of duties: a bug or prompt injection in orch can't bypass enforcement. Hard fault isolation around a security boundary is a valid reason to run a separate service.
**Rejected.** Enforcement inside orch. Either side importing the other.
**Tag.** [stated] · remind-me: mem_4c1e425c3d2b4903b3cd6f6aae5e37e6
**Key.** lel:726:orch-gateway-separate
**Source.** Mill Board card #726, comment by claude-cloud at 1793816610843 ms, "Decisions, Nano, 2026-11-04 (after spike #727)", D-b. Nano asked for it to be in this file in thread message cmsg_01WgyiM31RsSnZTgNuPX2VE42EMPL7EXicY3bTWURHCCkG (2026-11-04T23:51:22Z). Numbered D80 on PR #1496's branch; moved to D102 on 2026-11-05 because main assigned D78–D88 first.

## D103. Agents get mux-hosted ptys (supersedes D100; reopens D20 and D66)

**Decision.** Let Em Loose agents run in rusty_mux-hosted ptys. This supersedes D100's narrower "design only, doesn't decide that the mux hosts" framing, and deliberately reopens D20 and D66 (see their Reopened notes). The design follows six constraints:
1. **orch decides, mux hosts.** orch requests a block. The block's process is the jail launcher (rusty_sandbox plus #726's mounts, seccomp and token, D101), which execs the agent. The design specifies the mux API orch needs (open, kill, quiesce, process-group and slot-hold semantics) and shows that D20's guarantees still hold.
2. **The mux socket never enters the jail.** The `RUSTY_MUX_SOCKET` scrub stays in orch-bbp's SCRUB. The pty crosses the boundary; the control socket doesn't (D13).
3. **Human input is off by default.** Takeover is explicit, and every takeover and input batch is recorded on the board against the claim. The design proposes the BBP event shape as an ask-first schema change.
4. **Record vs. display.** The block shows orch's ACP `session/update` stream and the live pty. Evidence stays the witness diff and exec log plus the gateway log (D101). Mux scrollback is never evidence.
5. **Topology.** The mux server runs on BaileyAi (D53); rusty_term on Nano's desktop attaches remotely. The design checks whether rusty_mux supports remote attach and, if not, sizes the work.
6. **Scope.** #600 builders first, then lens sessions under #726. Codex stays a one-shot lens in v0 (D64).
Design first (D25): the design doc goes in `docs/projects/let-em-loose/` for Nano's approval; no code. Rusty Term owns the mux and term side, LEL owns orch launch, the jail and board events: one cross-project card, one PR per side.
**Why.** Nano wants agents to live in mux blocks, one environment instead of a watcher over logs. Keeping orch as the decider, the jail as the block's process and the witness as the only evidence keeps D13, D3 and D22 intact.
**Rejected.** A watch-only card after #600 or now (D100's options A and B). Treating mux scrollback or the ACP stream as evidence.
**Tag.** [stated] · remind-me: mem_833a5f7ac502415a90e7747bb23fc815 (supersedes mem_8d721e801bdf4d65a98e6035ad7c6b37 and mem_ed61a2115b084bbf8798285bcbeeac86)
**Key.** lel:mux:agents-get-mux-hosted-ptys
**Source.** Nano, thread message cmsg_01WgyiM31RsSnZTgNuPX2VE42EMPL7EXicY3bTWURHCCkG (2026-11-04T23:51:22Z), thread cmsg_01WgyiM31RsSnZTgNuPX2VE4KPB5ARgEwMpYqdBDPfmTDb: "Nano decision 2026-11-04: Option C [stated]. Agents get mux-hosted ptys. This deliberately reopens D20 and D66; log both as reopened, with this as the reason." Numbered D81 on PR #1496's branch; moved to D103 on 2026-11-05 because main assigned D78–D88 first.
**Approved 2026-11-05 (D104).** The design for this decision is approved; see D104.

## D104. The mux-hosted agent ptys design is approved (re-closes D20 and D66)

**Decision.** Nano approved `docs/projects/let-em-loose/mux-hosted-agents-design.md` with all eight §10 recommendations:
1. The design is approved as a whole. D20 is re-closed as "orch is the only executor; the mux hosts ptys" (§3). D66 is re-closed with the #600 builder launch through the mux `Launch`; its lease, sandbox, wall clock and mirror are unchanged.
2. The BBP takeover events in §5 are approved as shaped: `takeover_started`, `takeover_input` and `takeover_ended`, the human verb `takeover`, log schema 6 for claims tasks (earlier logs byte-identical), and `human_touched` on the candidate manifest and `claim_card`. This clears that ask-first schema change; scoring is unchanged and takeover doesn't count toward `min authors`.
3. Takeover input is recorded as-delivered with a hash chain, not write-ahead.
4. Mux §9 rows 1, 2, 3, 5 and 8 start now; 4 and 6 are deferred, with a viewer program for 6. This is also a Rusty Term decision: this is the LEL view, and the Rusty Term pack records its own.
5. A granted input lease survives rusty_term's reconnect grace until its `until`, and is refused on re-ask once revoked. This keeps rusty_term D44 and D46.
6. Remote attach: (a) Windows `--ssh` plus the C26 map entry `nano → orch-runner` first; (b) a mux client inside rusty_term later.
7. #600 starts `lel-jail` on rusty_sandbox, seccomp and cgroup; the #726 layers (overlay, gateway-only egress, the witness) come later behind the same `jail.json`.
8. Lens sessions in blocks (the D8 and D54 conflicts) are decided when #726 reaches lens sessions. Nothing changes now; lenses stay orch-side and one-shot.
Each side may now start its PR under card #732 (§11). Review and merge gates still apply.
**Build note (D105, D106).** Item 2's "log schema 6" is implemented as new claims-mode events under schema 5, with no bump (D105), and `human_touched` is derived in the fold (D106). The events, the verb, payloads, visibility and the byte-identical guarantee are as approved.
**Amended by D109.** Item 2's event set gains `takeover_closing`, and `takeover_ended` is host-only.
**Confirmed (reported) 2026-11-05.** dottiusmaximus reported on PR #1496 (https://github.com/Rusty-Mill/rusty_mill/pull/1496#issuecomment-6670572840, 2026-11-05T21:55Z) that Nano confirmed in the queue conversation: "1496 is approved". The reviewer counts this as fresh owner approval of the PR's mux-hosted agent design. It is a reviewer's relay, not Nano's own record, so the tag stays `[stated]` on the decision card below. The reviewer notes that the earlier decision cards weren't retrieved independently. The approval doesn't authorize a merge, a live launch, a host operation or spending, and review and CI gates still apply.
**Why.** Design first (D25) on D103. Both experts checked the design against the code. orch stays the only executor and the mux only hosts ptys, so D20's guarantees and D13 hold, and evidence stays the witness (D101). This closes OPEN_QUESTIONS Q13 (design §6: builder session files under `<dir>/builders/<claim>/<n>/`).
**Rejected.** Approving the design but holding the takeover schema for a separate review (option B). Changes first (option C). Write-ahead recording.
**Tag.** [stated] · remind-me: mem_fe60aaf6dae240b8b820ef918c6dca4f
**Key.** lel:mux:hosted-agent-ptys-design-approved
**Source.** Nano tapped option A "Approve as recommended" ("§10 items 1-8 take the recommended answers, the takeover schema change is approved, and each side can start its PR.") on decision card cmsg_01WgyiM31RsSnZTgNuPX2VE41VyDsxTHxmCrXAPxQnCpra ("Approve the mux-hosted agent ptys design with all eight recommendations?"), 2026-11-05T00:00:23Z, thread cmsg_01WgyiM31RsSnZTgNuPX2VE4KPB5ARgEwMpYqdBDPfmTDb. Numbered D82 on PR #1496's branch; moved to D104 on 2026-11-05 because main assigned D78–D88 first.

## D105. Takeover events go under claims log schema 5, with no schema 6 bump

**Decision.** `takeover_started`, `takeover_input` and `takeover_ended` (D104 item 2) are added as claims-mode events under `LOG_SCHEMA_CLAIMS` = 5 and written only when a takeover happens. There is no log schema 6. The event names, the human verb `takeover`, payloads and visibility are as approved; only D104's "log schema 6" wording changes.
**Why.** The design's aim was "earlier logs stay byte-identical; nothing is written when there's no takeover", and this meets it with a smaller change. #597 set the precedent under the accepted contract: `HoldoutExtended`, `HoldoutUpdated` and `holdout_digest` were added under schema 5 (addendum § Schema and API changes item 12). `Event` is an externally tagged serde enum with no catch-all, so a build that predates takeover refuses a log containing one rather than misreading it; the variant is the fence. A bump would make every new claims task unreadable to older builds even with no takeover, and `state.rs` takes claims mode from `schema == LOG_SCHEMA_CLAIMS`. Nano can't tell the two apart without reading the code.
**Rejected.** Log schema 6 for every new claims task. An open-time opt-in flag.
**Tag.** [default] · remind-me: mem_6385fa126f2349c09e26b166d4e2c5e7
**Key.** lel:mux:takeover-events-under-schema-5
**Source.** Caller question from the #732 LEL build, 2026-11-05; `rusty_bbp/src/event.rs` (`LOG_SCHEMA_CLAIMS`, `HoldoutExtended`, `HoldoutUpdated`), `src/state.rs`; bbp-claims-addendum.md § Schema and API changes items 1 and 12; D104. Numbered D83 on PR #1496's branch; moved to D105 on 2026-11-05 because main assigned D78–D88 first.

## D106. `human_touched` is derived in the fold, not stored

**Decision.** `human_touched` (D104 item 2) is computed by the claims fold and shown on `claim_card` and in the inbox; it isn't a stored field on the candidate manifest. A candidate submitted for claim c is `human_touched` when a `takeover_started` on c in the same epoch comes before it, whether or not that takeover is still active when the candidate is submitted. Scoring is unchanged, and a takeover doesn't count toward `min authors`.
**Why.** Nano sees the same flag either way, and a derived flag writes nothing to the log, so it can change without a schema change. Design §5 marks a candidate "stored from a session that had a takeover", so a takeover that ended before submission still counts; "active at submission" would miss it. Over-marking is the safe direction.
**Rejected.** A stored `human_touched` manifest field. Marking only candidates submitted while a takeover is active.
**Tag.** [default] · remind-me: mem_2d7809b2d14348079cb2974291b865d6
**Key.** lel:mux:human-touched-derived-in-fold
**Source.** Caller question from the #732 LEL build, 2026-11-05; mux-hosted-agents-design.md §5 "Effect on claims (v0)"; D104. Numbered D84 on PR #1496's branch; moved to D106 on 2026-11-05 because main assigned D78–D88 first.

## D107. `lel-jail` v0 launch contract: `orch-jail` crate, `jail.json` fields, setup order, tests

**Decision.** Implementation defaults for the v0 jail launcher inside the approved design (D104 item 7; mux-hosted-agents-design.md §2 step 5, §4, §8 item 1):
1. **Crate.** `crates/apps/rusty_orch/crates/orch-jail`: a lib with `JailSpec` (sans-IO parse and validate of `jail.json` through `rusty_serde`) and the bin `lel-jail`. orch-bbp's builder launch (slice 6) writes the spec with the same type. Design §11's "in orch-bbp" is the PR's work split, not a crate layout.
2. **`jail.json` v0 fields.** `cgroup` (absolute, under `/sys/fs/cgroup/`, no `..`), `status`, `cwd`, `read`, `write` (absolute roots; `cwd` inside a write root), `limits` {`cpu_secs`, `memory_bytes`, `file_bytes`, `open_files`, `processes`}, `env` (exact name/value list; any `RUSTY_MUX_*` name is refused, D13), optional `token` {`file`, `var`} (the file is outside every read and write root; `var` must not repeat an `env` name), `program`, `args`. Unknown fields are refused. There is no `sockets` field: v0 is fixed to `Sockets::InternetOnly`, so the file can't weaken it. Mounts, overlay and the network namespace are added later behind the same file (§8).
3. **Setup order.** Validate; require fds 3 and 4 open (fail closed); join the cgroup through `<cgroup>/cgroup.procs`; `close_range(5, max)`; read the token; hand off to `rusty_sandbox::run_helper` with `Sockets::InternetOnly` (the same profile the bbp_host harness uses, `rusty_bbp_host/src/confine.rs`). A failure before the handoff writes the status file, prints to stderr (the pty) and exits 125 (`SETUP_FAILED`). v0 unshares no namespaces: §2 step 5's namespace list is the #726 target, and D104 item 7 and §8 put those layers later.
4. **Tests in this slice.** Spec unit tests (happy path, relative path, unknown field, `cwd` outside the write roots, a `RUSTY_MUX_*` env name, a token file inside a root). A Linux integration test that runs the built `lel-jail` and checks the exec'd program sees only fds 0–4, `RUSTY_MUX_SOCKET` is unset and `socket(AF_UNIX)` fails, plus fail-closed when fd 3 or 4 is missing. It skips with a printed reason when no writable cgroup is available. §4's "the socket path doesn't exist" test waits for the mount namespace; in v0 Landlock keeps the path out of reach. Holder tests are slice 5.
**Known v0 gap.** `InternetOnly` allows any IPv4/IPv6 egress. 600-builder-session-design §4.3 wants none except the Claude endpoint, and the mirror makes dependencies offline. Enforcing that needs the network namespace, which D104 item 7 (newer and [stated]) defers with gateway-only egress. This is not a reversal.
**Why.** A separate crate keeps Linux and `rusty_sandbox` dependencies out of orch-bbp, which has none today, and gives the writer and the reader one type. Reusing the existing helper and socket profile is the most reversible choice inside the approved design.
**Rejected.** A bin inside orch-bbp. A configurable `sockets` field in v0.
**Tag.** [default] · remind-me: mem_bfc73721060b45989e4f32393bcd3c52
**Key.** lel:600:lel-jail-v0-launch-contract
**Source.** Caller question from the D104 slice 4 thread (`lel-jail` v0), 2026-11-05; mux-hosted-agents-design.md §2, §4, §8, §10 item 7; D101, D104; `rusty_sandbox/src/helper.rs`; `rusty_bbp_host/src/confine.rs`. Numbered D85 on PR #1496's branch; moved to D107 on 2026-11-05 because main assigned D78–D88 first.

## D108. #600 cgroup holder, slot hand-off and cgroup `quiesce`

**Decision.** Implementation defaults for slice 5, inside the approved design (D104; mux-hosted-agents-design.md §2 step 3, §3, §11):
1. **`bbp slot-hold --cgroup PATH`.** After stdin reaches EOF, the holder waits until `<cgroup>/cgroup.events` reads `populated 0`, not until a process group is empty. At `--deadline-ms` it writes `<cgroup>/cgroup.kill`, waits until the cgroup is empty, then exits. In this mode it never calls `killpg`.
2. **The slot fd.** The holder takes the slot itself through the existing `slots.rs` (one implementation of the slot and `capacity` contract; orch-bbp stays CLI-only and doesn't link `rusty_bbp_host`). orch, the holder's parent, takes a dup of that same open file description with `pidfd_getfd` (`rusty_libc`'s pidfd calls; SCM_RIGHTS over a socketpair would do the same), and orch passes it to the mux as fd 3 (§2 step 4, D107). The holder, orch's copy and the jail's copy then share one lock, as §3 requires.
3. **The cgroup directory.** A separate command, `bbp cgroup-open --dir D --task T --cgroup PATH`, writes the record `mod/<task>.cgroups/<name>` first, then creates the directory, as `groups.rs` records a group before anyone joins it. So no cgroup exists unrecorded after a crash. This replaces §2 step 2's "orch creates the cgroup", a wording detail: the cgroup still exists, empty, before `Launch`.
4. **`bbp quiesce`.** For an ended task (as today), it also settles `mod/<task>.cgroups/*`. A populated cgroup gets `cgroup.kill` and a wait. An empty one is removed with `rmdir`, then its record goes. Killing by path is safe without the holder because each path is unique (`lel/<task>/<claim>/<n>`). It answers `true` only once every recorded cgroup is gone (§3).
**Amended 2026-11-05 (PR #1507).** The record and `mkdir` step moved from the holder into `bbp cgroup-open`, with the same order. Slot-taking and the fd hand-off move to slice 6 with their first caller; until then the slot reaches `slot-hold --cgroup` on stdout. The hand-off uses `pidfd_getfd`, not SCM_RIGHTS. It needs ptrace-attach rights over the holder (Linux 5.6 or later; a Yama `ptrace_scope` of 2 or more refuses it), so a refusal must fail the launch closed, never launch without fd 3.
**Amended 2026-11-05 (PR #1496 review, Codex comment 4484772928).** Items 1, 3 and 4 above summarise an earlier draft; the merged contract (`rusty_bbp_host/src/cgroups.rs` `open`, `hold`, `settle`; README § Admission, the cgroup paragraph) wins:
- Item 3: `bbp cgroup-open` runs under the task's cgroup lock (`mod/<task hash>.cgroups.lock`) and refuses an ended task. It records the intent `mod/<task hash>.cgroups/<digest>`, then claims the path for the task with an exclusively created `mod/cgroup-owners/<digest of the path>` (drafted and renamed in whole, under one lock shared by every task; refused if any claim is the same path, an ancestor or a descendant), then creates the cgroup exclusively, and only then marks the record `created <inode>`. A failed open takes its record and claim back. The path must sit below `/sys/fs/cgroup/` and below this control directory's scope: exactly one name in it starts with `lel-store-`, the 16 hex digits `bbp cgroup-scope --dir D` prints. There is no `lel/<task>/<claim>/<n>` layout.
- Item 1: `slot-hold --cgroup` opens the cgroup once, after `cgroup-open`, and reads and kills through that handle; from the deadline it writes `cgroup.kill` at every look that doesn't find the cgroup empty. A failed read or kill is reported and retried, never taken for emptiness.
- Item 4: path uniqueness alone does not make a kill safe, and quiesce never kills by path. Under the same lock, it touches a cgroup only when the record is marked `created <inode>`, the task holds the claim, and the cgroup opened at the path has the recorded inode; it reads, kills and empties it through that pinned handle. Populated: `cgroup.kill`, answer stays `false`. Empty: its empty child cgroups (deepest first), then it, then the claim and record are removed. A bare intent, a record whose task doesn't hold the claim, or a cgroup made again at the path is left alone; the record is dropped, with the claim only if its task holds it, so a pre-existing or foreign cgroup is never killed or removed.
**Why.** The design shows the holder as "an evolution of `bbp slot-hold`" and lists cgroup-based `quiesce` in `bbp` (§11), so these flags are inside what Nano approved. The hand-off keeps §3's "fd 3 inherited into the jail" guarantee without a second slot implementation.
**Rejected.** orch-bbp linking `rusty_bbp_host` to call `Slots` in-process, which breaks its CLI-only boundary. Alternative B, where no slot fd goes into the jail: if the holder dies while the agent lives, the slot is freed early. That drops §3's kernel-lock guarantee and changes §2 step 4 and D107, so it would need Nano.
**Tag.** [default] · remind-me: mem_5de884e478294bc8bc17ad735a185d32
**Key.** lel:600:cgroup-holder-and-quiesce
**Source.** Coordinator question for D104 slice 5, 2026-11-05; mux-hosted-agents-design.md §2 steps 1–4, §3, §11; D104, D107; `rusty_bbp_host/src/slots.rs`, `groups.rs`, `moderator.rs` (`quiesce`), README § slots; amendment: merged `cgroups.rs` and README § Admission (cgroup paragraph), Codex review comment 4484772928 on PR #1496. Numbered D86 on PR #1496's branch; moved to D108 on 2026-11-05 because main assigned D78–D88 first.

## D109. Takeover end drains delivered input through `takeover_closing` (amends D104 item 2)

**Decision.** A takeover ends in two steps:
1. **`takeover_closing` {`takeover`, `reason`, `at`}.** New claims-mode event. The human's `TakeoverRelease` writes it with reason `released`; `Close` and `Cancel` write it for every live takeover, before the state change, with reason `turn_ended`. Input permission ends at `at`. There is no second release and no reopening.
2. **Drain.** The host may still record `takeover_input` until `min(until, closing at)` + `TAKEOVER_DRAIN_MS` (10 s).
3. **`takeover_ended`.** Host-only, written once, totals taken from the fold, with the reason the takeover closed with. A takeover with no `takeover_closing` ends `expired` (after `until`), `revoked`, `turn_ended` or `mux_lost`, never `released`. `bbp takeover-ended` accepts `--reason released` for a drained released takeover.
4. **Terminal guard.** On a Closed or Cancelled task, only those two host writes for a closing takeover pass (`claims_engine::closing_audit`); everything else is still refused as terminal.
Still under claims schema 5 (D105): no bump. D104's events, verb, visibility and hash chain are otherwise unchanged.
**Why.** Recording is as-delivered (D104 item 3), so input the mux had already delivered could be lost if the end was recorded before orch posted its last batches. Stopping input at `at` and giving the host a bounded drain keeps the audit complete and still fails closed.
**Rejected.** Release only asks orch to end the takeover (the human writes nothing). Accepting the loss.
**Tag.** [stated] · remind-me: mem_b63239cb445d4a49a078d1a45590fa4e
**Key.** lel:mux:takeover-closing-drain
**Source.** Nano tapped option A, "Add a takeover_closing event", on thread decision card `cmsg_01WgyiM31RsSnZTgNuPX2VE46GHhPehbN6LFb6VXZh2ZpE` at 2026-11-05T03:14:32Z. The card asked: "How should #1502 keep the last typed batch when you release a takeover or close the task?" Earlier, and first recorded: owner approval relayed by dottiusmaximus in a GitHub comment on PR #1502, https://github.com/Rusty-Mill/rusty_mill/pull/1502#issuecomment-6646844427 (2026-11-05T01:13Z), quoting the exchange: "#1502 needs a small audit-schema decision: delivered input can be lost when a takeover ends. I recommend adding a closing state that briefly drains already-delivered input before the host records the end. Are you okay with that?" Owner: "👍". Built in PR #1502 (head b5dd857e) and PR #1504 (head e4050a03). Numbered D87 on PR #1496's branch; moved to D109 on 2026-11-05 because main assigned D78–D88 first.

## D110. #704 slices 4–5: cleanup trigger and calibration build defaults

**Decision.**
- Slice 5 (calibration) goes before slice 4. It depends on no open definition, and Nano needs its numbers before the D88 time box ends.
- Calibration runs `erosion check` locally over git history, with no provider calls. The candidate is each merge commit and the base is its first parent. The report gives per-PR deltas, p50 and p90 for each metric, `unmeasured` counts (D45) and gaming signals (function count or total CC rising while concentration falls). It proposes p90 limits only; Nano sets them (D42, Q9, Q20). It goes in `docs/projects/let-em-loose/erosion-calibration.md` with the command that reproduces it, in slice 5's PR, and is then posted to Nano on card #704. Which PRs count as "human" is Q36; interim: report both sets and propose from the strict one.
- Slice 4 lives in `rusty_orch` as a `consolidate` module behind an opt-in `rusty_orch consolidate [--repo DIR] [--bbp CONFIG] [--execute]`, read-only plan by default like `maintain`. "Writes a `consolidate` intent" means orch opens a new claims task whose brief is the consolidate intent, through the host's existing `admin::open_claims_task`, with no origin header, `rules.toml` or host rule check (D55). That task's erosion profile enforces the D44 cleanup predicate whether or not ordinary limits are still report-only.
- The count leg counts accepted claims on claims tasks for the same repo, across tasks, since the last cleanup marker. Claims on consolidate tasks don't count. The marker is orch's write-ahead journal record of the last consolidate open or zero-floor skip. No new trigger fires while a consolidate task is open.
- With no approved limits (report-only), the half-limit leg can't fire, because D42 forbids inventing limits; only the count leg runs. How that leg is defined across two metrics, and what "cumulative erosion since the last cleanup" measures, is escalated to Nano (card #704 Open).
**Why.** These choices sit inside D42, D44, D45 and D55. They reuse the `maintain` opt-in pattern and the open path D55 keeps. Counting per repo matches D44's "since the last cleanup". A local git computation is not on the ask-first list.
**Rejected.** Building the half-limit leg on a guessed definition. Counting per task. Delivering calibration after the cleanup trigger.
**Tag.** [default] · remind-me: mem_2da162204bbf46b8a4020870babaadb5
**Key.** lel:704:cleanup-calibration-build-defaults
**Source.** Questions Q1–Q6 from the #704 slices 4–5 build thread, 2026-11-05; D42, D44, D45, D55, D88; card #704 Builds 4–5, Time box and Open; `rusty_orch` `src/maintenance.rs`; `rusty_bbp_host` `src/admin.rs` `open_claims_task`. Numbered D110 because open PR branches already use up to D109 (#1496). Its open question is Q36 because #1520 uses Q34–Q35.

## D111. #599 `rusty_orch lens record`: read-only calibration report command

**Decision.** Add a read-only subcommand, `rusty_orch lens record`, that prints one JSON line per lens: points, scored, pivotal and pivotal-with-ground-truth (D93's calibration report). It goes in a follow-up PR after #1520. Until then the record shows in each lens's context line (D92). The approval covers this one command only.
**Why.** D93 asks for a per-lens calibration report Nano can read, and no approved command fits it. A new public CLI command is on the ask-first list.
**Rejected.** The context line only, with no new command.
**Tag.** [stated] · remind-me: mem_55db68a49aa14242b83c4150dc738edf
**Key.** lel:599:lens-record-command
**Source.** Nano tapped option A, "Add lens record command", on decision card cmsg_014AqbZs58L565hMV2pFkCX524Ybpagn8gHAPAfimMrHys (thread cmsg_014AqbZs58L565hMV2pFkCX5Fiyuxpido5spNwMT4HHhuz), 2026-11-05T21:15:06Z (tap message cmsg_014AqbZs58L565hMV2pFkCX574JBwmkjmMnGTFTzf3n9MZ). Closes OPEN_QUESTIONS Q34.

## D112. #599 `claim_history` gains `min_authors`, golden results, evidence refs, reopen wakes and candidate provenance

**Decision.** One additive view change to `rusty_bbp::history`, shown by `bbp why`, in one follow-up PR after #1520. It changes neither the log nor the schema (like D77). It adds:
- `min_authors` on `Rule::AcceptLine` and `Rule::DropLine`;
- passed golden results;
- refs on evidence steps;
- on each reopen step, the items it woke;
- candidate steps;
- on each `Failing` step, the candidate its report tested.
The first three were approved first. The last three were added to Q35 after that card and approved on a second card. When the PR lands, all four of D92's interim limits end: golden truth is then tied to the candidate current at settlement and works on the `bbp why` path. Built in PR #1534. `Rule::AcceptLine` and `Rule::DropLine` carry the task's `min_authors`; evidence steps carry their refs as posted; status steps carry `woke`, the items moved into the new epoch as scoring, which is empty except on a reopen, and `holdout`, the report BBP's `holdout_truth` says speaks for the current code just after the change; `Failing` carries the candidate current when it fired; a `Candidate` step fires when a build candidate becomes current; and a `Report { report, candidate, passed, failed }` step records a final, non-error holdout report from a holdout candidate building the current code, under the task's holdout digest, on the current candidate's tested tree, using the engine's own `builds_current` and `on_tested_tree` checks. orch-lens takes golden truth only from the report a line settlement's status step names, doesn't read `Candidate` steps, requires these fields and refuses an older history (D92 interim limits, item 5) [default].
**Why.** It is view-only and closes every gap behind D92's interim limits in one change. Without the reopen wakes and candidate provenance, lines after a human reopen stay pivotal and golden truth can't be tied to the code current at settlement. It is a public API change, so it needed Nano's go-ahead.
**Rejected.** Only `min_authors`. No change, keeping the interim limits. Only the first three fields, keeping the interim limits for reopen wakes and golden provenance.
**Tag.** [stated] · remind-me: mem_2bc7a496ac8c4d8aa821bfa826891f85
**Key.** lel:599:history-track-record-fields
**Source.** Nano tapped option A, "All three fields", on decision card cmsg_014AqbZs58L565hMV2pFkCX5SMdL73WvySEECojiZphZ7g (thread cmsg_014AqbZs58L565hMV2pFkCX5Fiyuxpido5spNwMT4HHhuz), 2026-11-05T21:15:10Z (tap message cmsg_014AqbZs58L565hMV2pFkCX5HzE9svLMYjuftbZk7JHiYM). The card was posted 2026-11-05T18:20:06Z, before Q35's extension. The rest: Nano tapped option A, "Yes, same follow-up PR", on decision card cmsg_014AqbZs58L565hMV2pFkCX553uoAaCJ8FP2tp7oU1ibhQ ("Should the BBP history change also carry reopen wakes and candidate provenance?", same thread), 2026-11-05T21:35:55Z (tap message cmsg_014AqbZs58L565hMV2pFkCX5EJLCSPUm4HnqqKv3SmRqby). Closes OPEN_QUESTIONS Q35.

## D113. #704 slice 4: cleanup mode, consolidate inputs, split

**Decision.**
- **Cleanup mode.** D44's cleanup predicate is enforced by a new mode of the erosion bin: `erosion check --base COMMIT --cleanup`. It prints the same JSON line with mode `"cleanup"`. It exits 0 when `delta_concentration <= 0` and `delta_duplication <= 0` with at least one strictly negative, 1 when that fails, and 2 on a measure error. The host's `check_erosion_base` accepts `--cleanup` as a third exclusive mode, which takes no limits. `erosion_report_only` stays false for it, so the D88 ledger neither counts nor pauses it.
- **Consolidate inputs.** The consolidate open takes explicit files, as `bbp open` does: `--profiles`, `--roster`, `--claims` and `--holdout-profiles`/`--holdout-root`, plus `--repo`, `--repo-path` and `--work`. orch sets only the erosion profile's `--base`, and the pinned baseline through `bbp open --base`, to the head it measured. It refuses a profile set whose erosion profile is not in `--cleanup` mode. The brief is a fixed consolidate text. orch copies no other task's frozen set or holdout. Which holdouts a cleanup must keep green is Q37.
- **Split.** Slice 4a is the bin's `--cleanup` mode and host acceptance, with tests. Slice 4b is orch `consolidate`: the count leg and the zero-floor skip, a plan by default, and `--execute` opening through `bbp open --mode claims`.
- **Current levels.** For the zero-floor skip, orch runs `erosion check --base HEAD --report-only` in a clean checkout of HEAD and reads `concentration_bp.base` and `duplication_bp.base`. The bin gets no `levels` subcommand.
- **Repo identity.** A task's repo is `TaskOpened.repo`, the opaque string given to `bbp open --repo`. orch matches it exactly against its own `--repo` value, never against `--repo-path`.
**Why.** An extra mode in an in-repo bin adds to its argv without changing the BBP schema or any existing profile digest. It mirrors the existing modes and exit codes and keeps D82's exact-argv check. Explicit files match `bbp open` and avoid reusing a holdout written for another task's intent. Splitting the slice keeps each PR reviewable. The bin's base measurement already gives current levels.
**Rejected.** Copying the newest claims task's frozen set, holdout and roster. Enforcing cleanup through ordinary limits of 0 and 0, which would pass an unchanged candidate. A separate `levels` subcommand. Matching repos by path.
**Tag.** [default] · remind-me: mem_be0176f1b8094208ace97f73cbe425f0
**Key.** lel:704:cleanup-slice4-build-defaults
**Source.** Follow-up questions A–E from the #704 slice 4 build thread, 2026-11-05; D37, D44, D55, D82, D88, D110; `rusty_bbp_host` `src/profiles.rs` `check_erosion_base`, `src/bin/bbp.rs` `open`, `src/admin.rs` `open_claims_task`; `rusty_bbp` `src/event.rs` `TaskOpened`; `rusty_erosion` `src/main.rs`. Numbered D113 because main holds D110–D112. Its open question is Q37 because #1530 holds Q36.

## D114. #704 D44 cleanup half-limit leg: repo head vs last cleanup

**Decision.** D44's "cumulative erosion since the last cleanup reaches half the limit" is measured on the repo, not on claims. orch compares the current repo head with the base of the last cleanup, per metric. It writes the `consolidate` intent when either metric's change (concentration or duplication) reaches half that metric's approved limit. Human commits count too, not only accepted claims. The count leg (5 accepted claims, D44, D110) is unchanged. With no approved limits (report-only, D42) the half-limit leg still can't fire; it starts once limits are recorded (D115).
**Why.** Comparing the head with the last cleanup's base catches all drift, including work done outside claims.
**Rejected.** B: summing each accepted claim's erosion delta per metric, which misses work outside claims. C: leaving the leg undefined until Q20.
**Tag.** [stated] · remind-me: mem_fa9c239378ce45ec8e915338f5b3803c
**Key.** lel:704:cleanup-half-limit-measure
**Source.** Nano tapped option A "Repo head vs last cleanup" ("Fire when either metric's change since the last cleanup's base reaches half that metric's approved limit. Human commits count too.") on decision card `cmsg_014AqbZs58L565hMV2pFkCX54vFUN7KN9gHbTjLFBEB4an` ("How should erosion cleanups count “half the limit” across the two metrics?") in the #704 build thread, 2026-11-06T00:58:51Z. Settles the leg D110 left escalated (card #704 Open). Numbered D114 because this branch holds D113 and main holds D112.

## D115. #704 approved erosion limits: a human-only host record

**Decision.** Once report-only ends, approved erosion limits are only those recorded by a human command: a human-only host record of Nano's approved limits. A claims open whose erosion profile enforces limits must use exactly the recorded numbers, or it is refused; an enforcing open with no record is refused too. This resolves Q20 (option b) and replaces D88's interim "enforcing opens are unchanged".
**Why.** D42's "don't invent limits" holds only if an agent opening a task can't pick the numbers, as with D37's calibrate-then-Nano-approves.
**Rejected.** Treating any limits in the task's profile as approved: whoever opens the task writes the limits, and nothing checks them.
**Tag.** [stated] · remind-me: mem_69f9310316bf4c9693a3ab8dc1235db9
**Key.** lel:704:approved-erosion-limits-record
**Source.** Nano tapped option A "Recorded approved limits" ("Only a human command records your approved limits. A task that enforces limits must use exactly those numbers, or it is refused.") on decision card `cmsg_014AqbZs58L565hMV2pFkCX5UNTzdY4w4xdKPk1naZzrRp` ("Decide what counts as approved erosion limits once report-only ends") in the #704 build thread, 2026-11-06T00:58:58Z. OPEN_QUESTIONS Q20; D42, D88. Numbered D115 after D114.

## D116. A GUI shows the LEL stage strip as a derived read-only display and the product intent as a read-only brief

**Decision.** (1) A GUI's 5-stage strip (Intent, Skeletons, Under attack, Shadow run, Living app) is a derived, read-only display the GUI computes from claim and evidence facts. BBP and orch gain no stage field. (2) The shared product-intent layer is out of scope: the GUI shows the task brief read-only, and intent versioning stays on #598.
**Why.** D1, D9 and D27 say there is no stage state machine, so a derived view keeps them intact and needs no ask-first change. #598 already holds intent versioning and nothing is designed.
**Rejected.** A real product-level stage field in orch/BBP (reopens D1/D9). A structured, versioned, GUI-editable product-intent object now.
**Tag.** [stated] · remind-me: mem_28731854176542ec9a596164bb9ef8a9 (written by the rusty_term design expert; records Nano's AskUserQuestion answers, 2026-11-04; it is not LEL-tagged)
**Key.** lel:general:gui-stage-strip-derived-intent-readonly
**Source.** Nano via AskUserQuestion, 2026-11-04, relayed by the coordinator and verified in mem_28731854176542ec9a596164bb9ef8a9. Asked in the LEL full-app SVG review (README § Full-application interface, D1, D9, D27, D35, #598). Numbered D116 (first written as D83 on PR #1510) because main assigned D83 to #599 and holds D84–D115; its questions are Q38–Q43 for the same reason.

## D117. #704 `rusty_orch consolidate`: public subcommand approved as built in PR #1539

**Decision.** Add the public subcommand `rusty_orch consolidate --dir BBP --repo NAME --repo-path DIR [--erosion-bin BIN] [--execute ...]` as built in PR #1539 (#704 slice 4b). It prints a read-only JSON plan by default; `--execute` opens the D44 cleanup task through `bbp open --mode claims` with the explicit inputs D113 lists. The approval covers this one command only, as D111 did for `lens record`.
**Why.** A new public CLI command is on the ask-first list. D110 and D113 sketched it only as `[default]` build detail, and D58 doesn't list it, so #1539 was held until Nano approved it.
**Rejected.** Keeping #1539 unmerged until the command was redesigned or folded into an existing approved command.
**Tag.** [stated] · remind-me: mem_8f133032fecd47309bfe12f509c99e1f
**Key.** lel:704:consolidate-command
**Source.** Nano, "Merge #1539", in the rusty-mill project chat, message `cmsg_01Y8j3ihrYW8jedySti37M9c5Rt2WYjUHub7YUaW8eVWzN`, 2026-11-06T11:17:42Z, replying to `cmsg_01Y8j3ihrYW8jedySti37M9cGghHU2QCamzntkSrJXcLYB` ("Should I merge #1539 as built?", which named the new public command `rusty_orch consolidate`). That is option a of the expert's escalation (approve this one command as built). D58, D110, D111, D113.

## D118. #704 D115/D114 build: the limits record, its command, the open check and orch's half-limit leg

**Decision.**
- **Human-only.** As with D37's golden `SIGNOFF`, "human-only" means only a dedicated host CLI subcommand writes the record, and the record sits in the host data dir, outside every sandbox and the Agent API. No `admin` library function, MCP tool or orch code path writes it. Each record carries a required `--source`: an attributable reference to Nano's approval (decision card or thread message id, card comment, or a GitHub comment by `baileyrd`). The host doesn't verify the reference, prompt on a TTY or check a signature. Like D37's golden sets, this records an attestation and doesn't authenticate who wrote it: anyone who can run host commands as the operator can write a record, just as they can run `bbp human` or rewrite task logs (D88). Nano approved this trust model and both commands as built (D119).
- **Command and record.** `bbp erosion approve --dir D --concentration-bp N --duplication-bp M --source REF [--as WHO]` (N, M non-negative integers; REF non-empty). It appends one JSON line `{"at_ms","concentration_bp","duplication_bp","by","source"}` to the append-only `<dir>/erosion/limits.log` while holding an exclusive lock on `limits.log` itself (`src/erosion_limits.rs`). The newest line wins, and a later approval replaces an earlier one only for opens after it. If any line is malformed, enforcing opens are refused, so a bad record fails closed. `bbp erosion limits --dir D` prints the newest record as JSON, or `null`.
- **Open check.** In `open_claims_task`, a profile set whose erosion profile enforces (both `--max-*-bp`) is refused with `bad_claims_config` and the detail `erosion: ...` when there is no record, or when its limits differ from the newest record. Report-only, `--cleanup` and `"off"` are not checked. There is no new BBP code and no schema bump. D115 is Nano's approval of this behaviour, so no separate D25 design approval is needed; review and merge gates still apply.
- **orch half-limit leg (D114), not built in PR #1543.** A default for a later PR, which will also change `rusty_orch consolidate` (on main since #1539; remind-me `lel:704:half-limit-leg-build-details` holds that slice's finer defaults). `rusty_orch consolidate` reads the limits through `bbp erosion limits` (its runner and `--bbp-bin`, default `bbp`), never by parsing the file itself. The journal gets a serde-default field `last_base: {commit, concentration_bp, duplication_bp}`. It is set when an open settles (using the levels measured for that open) and on a zero-floor skip, the same events that move D110's marker. When limits are recorded but the journal has no `last_base` yet, the first `--execute` run stores HEAD's levels as the start and does not fire. The leg fires when `2 * (head - last_base) >= limit` for either metric (integer arithmetic, no rounding). With no record, the leg is reported as inactive and only the count leg runs.
**Why.** The D37 golden sign-off model and D88's host-ledger trust boundary are already accepted. Reusing `bad_claims_config` avoids a BBP schema change. Keeping the record's only parser in the host crate keeps orch from depending on the file format. A serde-default journal field reads existing journals unchanged.
**Rejected.** A TTY confirmation, which an operator-side agent with a pty passes anyway. A cryptographic signer, which D37 keeps out of the v0 contract and which would need Nano to hold a key. orch reading `limits.log` directly. Firing the leg against an implicit zero base before the first cleanup.
**Tag.** [default] · remind-me: mem_2f2f02f97ac4454b8886064d4e814fa8
**Key.** lel:704:approved-limits-record-build-defaults
**Source.** Builder questions 1–4 from the #704 D115 build slice, 2026-11-06; D37, D88, D110, D113, D114, D115; addendum § Golden sets; `rusty_bbp_host` `src/admin.rs` `open_claims_task`, `src/erosion_window.rs`, `src/golden.rs`, `src/erosion_limits.rs`; `rusty_orch` `crates/rusty_orch/src/consolidate.rs` (on main from PR #1539). Built in PR #1543 except the orch half-limit leg. Numbered D118 because main holds D117 (PR #1542); first written as D117.

## D119. #704 `bbp erosion approve` and `bbp erosion limits`: both commands and their trust model approved as built in PR #1543

**Decision.** Add the two public host subcommands as built in PR #1543 (D118): `bbp erosion approve --dir D --concentration-bp N --duplication-bp M --source REF [--as WHO]`, which records Nano's approved limits, and `bbp erosion limits --dir D`, which prints the newest record as JSON, or `null`. Accept their trust model: any process running `bbp` as the operator can record limits, and the `--source` reference to Nano's approval is required but not checked. This is the same model as D37's golden sign-off, and every record names its approval, so it can be audited. The approval covers these two commands only, as D111 and D117 did.
**Why.** D115 approved the behaviour (a human-only host record that enforcing opens must match) but not a second new public command or a "human-only" that is an unchecked attestation. A new public CLI command is on the ask-first list, so #1543 was held for Nano.
**Rejected.** Option 2: approve only the record command, drop `limits`, and have later orch work read the record another way. Option 3: hold #1543 and make limits a committed file Nano reviews in a PR.
**Tag.** [stated] · remind-me: mem_3af56d3beda444f0b68cecbad92b12f4
**Key.** lel:704:erosion-limits-commands-trust
**Source.** Nano, "1, approve as built", in the rusty-mill project chat, message `cmsg_01Y8j3ihrYW8jedySti37M9cH5a9PiPyhKBCrdwb9eatkm`, 2026-11-06T14:20:02Z, replying to `cmsg_01Y8j3ihrYW8jedySti37M9cEocaiXtqjs3qXh2GXqe2c2` (14:15:18Z), which asked for his call on #1543's second new command (`bbp erosion limits`) and its unchecked source. That is option a of the expert's escalation (approve both as built). D37, D111, D115, D117, D118.


## D120. #704 slice 4d: half-limit leg firing rule, measuring, `--bbp-bin` and pending base

**Decision.** Refines D118's orch half-limit leg (D114):
- **Firing rule.** Per metric, `change = head - last_base`. The leg fires when, for either metric, `change > 0 && 2 * change >= limit` (integer arithmetic). A zero or negative change never fires it. For limits of 1 bp or more this is the same as D118's literal rule.
- **Measuring.** When limits are recorded, every run measures HEAD (one `erosion check --report-only`) to evaluate the leg, even when the count leg isn't due. With no record, nothing is measured and the run keeps today's count-only wait.
- **`--bbp-bin`.** A top-level option in both plan and `--execute` modes, default `bbp`. Plan mode runs only the read-only `bbp erosion limits --dir D`.
- **Pending base.** The journal's `pending.base` becomes `{commit, concentration_bp, duplication_bp}`, so settling an open sets `last_base` from the levels measured for it. No compatibility shim: slice 4b's journal format isn't on main.
**Why.** D44 and D114 fire on cumulative *erosion*: a change that is zero or an improvement isn't erosion. Read literally, D118 with a 0 bp limit would reopen a cleanup on every run once the last one closed. Measuring is a local git computation with no provider calls. orch still reads limits only through the host CLI (D118). The journal format hasn't shipped.
**Rejected.** D118's literal `2 * change >= limit` with no sign check. Measuring HEAD on every run when no limits are recorded. Keeping `--bbp-bin` `--execute`-only, which leaves plan mode unable to report the leg.
**Tag.** [default] · remind-me: mem_23a36220effe41ffbc6f50f5fe6f6cdc
**Key.** lel:704:half-limit-leg-build-details
**Source.** Builder questions 1–4 from the #704 slice 4d build, 2026-11-06; D44, D110, D114, D118; `rusty_orch` `src/consolidate.rs` (branch `slice4d`). Numbered D120 after D119.

## D121. #704 slice 4d: `rusty_orch consolidate` half-limit leg changes approved as built in PR #1544

**Decision.** Accept the changes PR #1544 (#704 slice 4d, D114, D118, D120; ADR 0024) makes to the public `rusty_orch consolidate` command, as built: `--bbp-bin` now works in plan mode as well as with `--execute`; the JSON report gains four fields, `trigger`, `half`, `limits` and `last_base`; and plan runs now call the read-only `bbp erosion limits --dir D`. The approval covers these changes to this one command only, as D117 and D119 did.
**Why.** D117 approved `rusty_orch consolidate` as built in PR #1539. Changing a public command's flags and its JSON output is a public API change, on the ask-first list, and D120 set these details only as `[default]` build detail, so #1544 was held for Nano.
**Rejected.** Holding #1544 until the leg was redesigned, or keeping `--bbp-bin` `--execute`-only, which D120 already rejected because plan mode could not report the leg.
**Tag.** [stated] · remind-me: pending
**Key.** lel:704:consolidate-half-limit-leg-approval
**Source.** Nano, "Merge both", in the rusty-mill project chat, message `cmsg_01Y8j3ihrYW8jedySti37M9c5BJB8MJVcpTVXKHfp9P2zP`, 2026-11-06T15:47:26Z, replying to `cmsg_01Y8j3ihrYW8jedySti37M9cGcSaVAgBosLz316zUmS6k2` (15:14:33Z), which named these changes to `rusty_orch consolidate` and recommended merging #1544 (its other PR, rusty-remind-me #376, is not a Let Em Loose item). PR #1544 merged through the merge queue on 2026-11-06, merge commit `44cd63c7`. D114, D117, D118, D119, D120; `rusty_orch` `crates/rusty_orch/src/consolidate.rs`, `docs/adr/0024-consolidate-half-limit-leg.md`. Numbered D121 after D120.

## D122. Mill Board `external_refs`: observation-only references for non-Orch execution, approved in principle

**Decision.** Mill Board cards may carry observation-only `external_refs` for Codex Cloud and other execution that does not run under Orch (shape proposed: provider, ref, url, observed_at, state). Rusty Orch stays the execution authority, and `ExecutionLink` (written only by Orch) is unchanged. Mill Board must not use external observations to dispatch, retry, lease, fence or resume workers. This is approval in principle only. Before implementation, the schema and API contract must be finalized, covering validation, scoped authorization, idempotency, stale-observation handling, compatibility, retention and rollback. The S5 implementation needs its own design approval (D25) before any code, and the ask-first gates for the card schema and MCP API change still apply to that design.
**Why.** In production 0 of 1,425 cards have `execution` set, and about 63 cards record Codex Cloud thread and turn ids only in comments, so reconciling the Board, GitHub and cloud workers is hard. A pointer-only field fixes that without building a second orchestration engine inside Mill Board, which keeps D20/D104 (orch is the only executor) and the boundary in `rusty_orch/ARCHITECTURE.md` § Factory execution boundary.
**Rejected.** Keeping cloud execution ids in comments only. Requiring these workers to run under Orch so they get a real `ExecutionLink`.
**Tag.** [stated] · remind-me: mem_61e819e5a00743a3b802546c4e6f171b
**Key.** lel:general:mill-board-external-refs-observation-only
**Source.** Nano, Mill_Board project thread, message `cmsg_017MoLbtrGAAcyE1PVPZXuN6FBsfep5KMARhZchV2QDExC`, 2026-11-10T16:06:48Z ("Approve S5 option A in principle. Use observation-only `external_refs` for Codex Cloud and other non-Orch execution references. Preserve Rusty Orch as the execution authority. ..."), answering the expert's escalation from the Mill Board retrieval and execution-state scalability assessment. D20, D104, D25; `crates/apps/rusty_orch/ARCHITECTURE.md`. Numbered D122 after D121.  Design card: #1432 (S5, under #19); its design defaults are D123.

## D123. #1432 S5 design defaults: ADR location, providers, capability, summary flags, card_take

**Decision.** Inside D122, the #1432 design uses these defaults. (1) The design is the board ADR `crates/apps/rusty_mill_board/docs/adr/0006-external-execution-refs.md`, committed in its first draft PR; this pack points to it from D122 and here rather than keeping a second copy. (2) `provider` is a fixed set (`codex-cloud`, `github-pr`, `github-actions`) that never includes `orch` or `bbp`. Refs may sit next to an `ExecutionLink`, because a link can bind after refs exist and nothing reads refs. (3) `card_execution_observe` reuses the Build capability plus project scope, the same as `card_execution_link`; no new capability. (4) ~~Summaries keep `has_execution` meaning an `ExecutionLink` only, and add a separate `has_external_refs`; never one flag for both.~~ Superseded by D126 (Nano, 2026-11-12): summaries carry neither flag; readers who need an `ExecutionLink` or refs call `card_get`. One flag for both stays rejected. (5) `card_take` never skips, reorders or refuses a card because of its refs; the taken card carries `external_refs` as `card_get` does, with no extra take-time warning field.
**Why.** D122 makes Orch the only execution authority and forbids dispatch from observations, and `card_take` choosing a card is dispatch. One flag for link and refs would let a reader mistake an observation for an Orch execution. Reusing Build and the existing file layout adds nothing that needs an ask-first gate. A board ADR in the repo meets D94's purpose, which is that designs survive between sessions.
**Rejected.** A copy of the design under `docs/projects/let-em-loose/`. `orch` or `bbp` providers. A new `observe` capability, which changes roles and the API, so it would go to Nano. A single `has_execution` covering both. Skipping a card whose ref says `running`.
**Tag.** [default] · remind-me: mem_312ce716599d4b89aec9c5fec11d0b73
**Key.** lel:1432:external-refs-design-defaults
**Source.** let-em-loose-expert, answering the #1432 design session, 2026-11-11. D122, D94, D25, D20/D104; `crates/apps/rusty_orch/ARCHITECTURE.md` § Factory execution boundary; `crates/apps/rusty_mill_board/src/board.rs` (Build check on the execution ops). The whole design, with these defaults, still goes to Nano for approval under D25 before any code. Numbered D123 after D122.

## D124. #1432 S5 external refs design (ADR-0006) approved as written for implementation

**Decision.** The #1432 S5 design, ADR-0006 `crates/apps/rusty_mill_board/docs/adr/0006-external-execution-refs.md` in draft PR #1618, is approved as written for implementation, with the D123 defaults. This is the D25 design approval, and it clears the design's ask-first items: the card schema field `externalRefs` and the new MCP and HTTP op `card_execution_observe`. The work is built as two implementation cards: PR 1 keeps unknown board-document fields, then PR 2 adds external refs, each through review and CI. The approval settles what to build, not merging: review and merge gates still apply.
**Why.** D122 approved observation-only `external_refs` in principle and required the schema and API contract to be finalized and separately approved before code. ADR-0006 covers the points D122 listed (validation, scoped authorization, idempotency, stale observations, compatibility, retention, rollback), keeps Orch the only executor (D20/D104), and its two-PR order keeps rollback lossless.
**Rejected.** Approving with changes (revising ADR-0006 in PR #1618 before any code), and holding the design PR as a draft with nothing built; both were options on the same card.
**Tag.** [stated] · remind-me: mem_b512d055f0f3463aaa8068bec48916c5
**Key.** lel:1432:external-refs-design-approved
**Source.** Nano tapped "Approve as written" on decision card `cmsg_017MoLbtrGAAcyE1PVPZXuN6LV2exssuE5oC7WWqu8rWaa` ("Approve the S5 external refs design in PR 1618 for implementation?"; option: "I create the two implementation cards and build PR 1 (keep unknown doc fields), then PR 2 (external refs), each through review and CI."), decide event `cmsg_017MoLbtrGAAcyE1PVPZXuN64ukBebuUh4oWNpEUyf7mgw`, 2026-11-11T23:48:48Z, Mill_Board project thread. Confirms D122 and D123. D25, D20, D104. Numbered D124 after D123. D123 item (4), the summary flags, was later dropped by Nano: see D126.

## D125. #1432 S5 clarifications: `unknown` refs not evictable, S5a document readback, refs on the domain `Card`

**Decision.** Inside D124 (ADR-0006), three clarifications. (1) A ref in state `unknown` counts as not evictable, the same as a live ref, because the observer could not tell. The ninth ref replaces the terminal ref (`succeeded`, `failed`, `cancelled`) with the oldest `observed_at_ms`, and is refused if no ref is terminal. (2) S5a keeps unknown board-document fields by reading the stored document back before each rewrite. (3) In S5b, `external_refs` sits on the domain `Card` the same way `execution` does. This entry does not settle the summary flags in D123 item (4); that question went to Nano after the PR #1618 review.
**Why.** ADR-0006 defines `unknown` as neither live nor terminal and only says a live observation is never evicted. Refusing is the conservative reading: an observation that may still be running is never dropped, and the rule can be relaxed later. Readback and the domain `Card` placement follow the existing `execution` pattern and are implementation detail inside the approved design.
**Rejected.** Evicting `unknown` refs like terminal ones, which could drop a ref for work that is still running.
**Tag.** [default] · remind-me: mem_fbced0cfdbde43ee89a9f08dc4da4ef2
**Key.** lel:1432:external-refs-unknown-eviction-and-doc-preservation
**Source.** let-em-loose-expert, answering the PR #1618 review follow-up (branch `claude/project-thread-ofbftl`, head 85e0c449), 2026-11-12. ADR-0006 `crates/apps/rusty_mill_board/docs/adr/0006-external-execution-refs.md` (state table, ninth-ref rule); D124, D123. Numbered D125 after D124.

## D126. #1432 S5: card summaries drop `has_execution` and `has_external_refs` (supersedes D123 item 4)

**Decision.** Card summaries gain neither `has_execution` nor `has_external_refs`. A reader that needs a card's `ExecutionLink` or its external refs calls `card_get` (or uses the full `board_list` view), which already carry them. This replaces D123 item (4) only; the rest of D123 and D124's approval of ADR-0006 stand, narrowed by this change.
**Why.** Under S1, card summaries are built from task metadata and never read the board documents, so neither flag can be computed. The PR #1618 review found this after D124's approval. Nothing uses the flags today, so dropping them is the smallest change and only narrows what Nano approved.
**Rejected.** Keeping the flags as two reserved task tags kept in step with each card's document, which needs a second write per observation that can be partly saved, plus a new reserved-tag rule. Keeping them by reading every board document for summaries, which undoes the S1 speedup. A single flag for link and refs stays rejected (D123).
**Tag.** [stated] · remind-me: mem_a077a68ee1884f3ab39ac805e39850dc
**Key.** lel:1432:external-refs-summary-flags-dropped
**Source.** Nano chose option A, "Drop both flags", on decision card `cmsg_017MoLbtrGAAcyE1PVPZXuN62KVkvMfLiGbUKg62y2EiuZ` ("Drop the two summary flags from the external refs design?"), 2026-11-12T00:13:55Z, Mill_Board project thread `cmsg_017MoLbtrGAAcyE1PVPZXuN6Ek2MsYp9T23hpHFtsj2DWp`, answering the expert's escalation from the PR #1618 review (D125). Supersedes D123 item (4); narrows D124. ADR-0006 `crates/apps/rusty_mill_board/docs/adr/0006-external-execution-refs.md` still lists the flags and needs the matching edit in PR #1618. Numbered D126 after D125.

## D127. Design freeze: no new Let Em Loose design until its first run

**Decision.** Let Em Loose is under a design freeze until its first run. No new design is added to the project until then; work goes into building and running what is already approved. This is a scope rule and changes no existing design or decision.
**Why.** It is the project's own rule: build the slice before designing more.
**Rejected.** Continuing to add designs before the first run.
**Lifted by D164 (2026-11-18): smoke-7 is the first run under D143.**
**Tag.** [stated] · remind-me: mem_1bae9f1358544fc3a517998b983b896a
**Key.** lel:general:design-freeze-until-first-run
**Meaning amended by D143 (2026-11-17): the first run is a clean #1166 pass.**
**Source.** Nano, Millwright project thread message `cmsg_01632CBKCXsWe6RyFWvE9dovLW5oB3NEACWHuvMnsptmYP`, 2026-11-12T00:34:16Z: "Let Em Loose is under a design freeze until its first run. Add no design there." Answers `docs/projects/millwright/QUESTIONS.md` L9. Numbered D127 after D126.

## D128. M1 committed: #758 items 1-3, #1123, #1124, #1166 to Ready; #1092 waits for the BaileyAi host; #754 out

**Decision.** Let Em Loose's first milestone (M1) is committed. #758 (items 1-3 only), #1123, #1124 and #1166 go to Ready. #1092 (deploy layout) follows only once Nano confirms the BaileyAi host (Millwright L2). #754 (the MS-01 probe) leaves M1. #758 item 4 (the new Survey and Intent lenses) moves to its own card for after the first run. "Go" in the same message means go on Millwright L0. It is not read as Millwright L2: it does not authorise deploying on BaileyAi. Deploying stays ask-first, so #1092 stays in Backlog until Nano answers L2. That reading is the caller's, recorded here at its request; Nano's words do not mention L2.
**Why.** It is the smallest card set that reaches the first run, which the design freeze (D127) puts ahead of any new design. New lenses are new design, so they wait. Deploying on BaileyAi is ask-first and the escalation kept it as a separate question (L2), so a "Go" on L0 doesn't answer it.
**Rejected.** The other L1 options put to Nano. Reading "Go" as L2 approval to deploy.
**Tag.** [stated] · remind-me: mem_0ccb1eaa4d394c5c98ac572bbb8a8adc
**Key.** lel:general:m1-commit-cards
**Source.** Nano, Mill thread message `cmsg_01632CBKCXsWe6RyFWvE9dov81m5TvtNmj43NpXMCV23tc`, 2026-11-12T19:22:57Z, project thread `cmsg_01632CBKCXsWe6RyFWvE9dov5LxGS7LhnBwjYdXbAXf7nm`, verbatim: "Go, a, a, a". "Go" answers Millwright L0 and the first "a" answers L1, option a, as the caller put it to Nano from the expert's escalation earlier on 2026-11-12. D127. The board moves are the caller's; this entry moves no card. Numbered D128 after D127.

## D129. Lost designs not needed for M1; #1402 builder-session rebuild waits until after the first run

**Decision.** M1 needs neither lost design, 598-gaps-design.md or 600-builder-session-design.md. For #598 the gaps design is covered by `598-build/PLAN.md` and D86, which stand as the build basis. #1402 (rebuilding the builder-session design) waits until after the first run. "Lost designs rebuilt" drops from Millwright L0's done-when. This closes OPEN_QUESTIONS Q16 and defers Q44's rebuild to #1402.
**Why.** No copy of either file exists. M1 can run on the approved decisions (D59, D62, D63–D66, D86, D104), and rebuilding a design before the first run cuts against the design freeze (D127).
**Rejected.** Rebuilding either design before M1. Holding M1 until a copy turns up.
**Tag.** [stated] · remind-me: mem_46ee7dd7b8b942a3a62d5edc4a1b7fdd
**Key.** lel:general:lost-designs-not-needed-for-m1
**Source.** Nano, Mill thread message `cmsg_01632CBKCXsWe6RyFWvE9dov81m5TvtNmj43NpXMCV23tc`, 2026-11-12T19:22:57Z, project thread `cmsg_01632CBKCXsWe6RyFWvE9dov5LxGS7LhnBwjYdXbAXf7nm`, verbatim: "Go, a, a, a". The second "a" answers Millwright L7, option a. D86, D127; OPEN_QUESTIONS Q16, Q44. Numbered D129 after D128.

## D130. M1 design steps approved as one batch of runbooks and config (changes D25 for M1)

**Decision.** M1's design steps are approved as one batch of runbooks and config: #1092 deploy layout, #1123 prompts and model bindings, #1124 spend report and #1166 smoke-run plan. They count as runbooks and config, not new design under D127. This changes D25 (design first on every card) for M1 only: these four cards need no separate per-card design approval before work starts. It approves no ask-first item. Deploying on BaileyAi still waits for Millwright L2 (D128). Paid model calls, schema or API changes and the other ask-first items still go to Nano. Review and merge gates are unchanged.
**Why.** These steps run what is already approved. Treating each one as a new design would stall M1 under the freeze (D127), and one batch approval keeps Nano's sign-off without a design per card.
**Rejected.** Per-card D25 design approval for each M1 step. Treating them as new design under D127.
**Tag.** [stated] · remind-me: mem_970ab68163f14d498f99ddf976739c5f
**Key.** lel:general:m1-design-steps-batch-approved
**Source.** Nano, Mill thread message `cmsg_01632CBKCXsWe6RyFWvE9dov81m5TvtNmj43NpXMCV23tc`, 2026-11-12T19:22:57Z, project thread `cmsg_01632CBKCXsWe6RyFWvE9dov5LxGS7LhnBwjYdXbAXf7nm`, verbatim: "Go, a, a, a". The third "a" answers Millwright L8, option a. Changes D25 for M1; D127, D128. Numbered D130 after D129.

## D131. Deploy on BaileyAi authorised for M1 (protocol host, orch, Mill Board inbox); BaileyAi confirmed as host; #1092 released

**Decision.** Nano authorised deploying the protocol (BBP) host, orch and the Mill Board inbox on BaileyAi, and confirmed BaileyAi as the host (Millwright L2). This confirms D53 and lifts the D128 hold on #1092 (deploy layout). The MS-01 probe #754 stays out of M1 unless BaileyAi turns out to lack headroom. Nano also reported moving #758, #1123, #1124 and #1166 from Backlog to Ready (D128's gate); the Millwright thread agent then reported all four in Ready. #1092's dependency link on #754 still has to be removed on the board.
**Why.** Deploying on BaileyAi is ask-first (D128); this is Nano's answer, and BaileyAi is where D53 already put everything.
**Rejected.** MS-01 probe #754 as a precondition for M1.
**Tag.** [stated] · remind-me: mem_1592b43423844f33b090418ac5b6c46d
**Key.** lel:general:m1-baileyai-deploy-authorised
**Source.** Nano, Millwright project thread message `cmsg_01632CBKCXsWe6RyFWvE9dovEkaSZSapKW11x6su2fCt41`, 2026-11-13T23:27:31Z, project thread `cmsg_01632CBKCXsWe6RyFWvE9dov5LxGS7LhnBwjYdXbAXf7nm`, verbatim: "1. Done 2. Yes BaileyAi". Answers Millwright L2. Confirms D53; releases D128's hold; D127, D130. Not lifted: #1092's design is approved only as D130's batch; work on Nano's machines goes as a paste-ready prompt (D94); paid model calls, schema or API changes and the other ask-first items still go to Nano; review and merge gates unchanged. Numbered D131 after D130.

## D132. #1092: Mill Board runs as its own user (`mill-board`) on BaileyAi, reaching `bbp` through a narrow sudo rule

**Decision.** On BaileyAi Mill Board runs as its own system user `mill-board`, not `orch-runner` and not `baileyrd`. Its state and `board.env` live in `/var/lib/mill-board` (mill-board 0700, `board.env` 0600), so `orch-runner` and the Claude harness running as it cannot read `RUSTY_TICK_TOKEN` or the board's agent tokens. `/srv/lel/bbp` and `master` stay `orch-runner` 0700/0600. The inbox reaches `bbp` through a root-owned wrapper `/usr/local/libexec/mill-board-bbp` (`exec sudo -n -u orch-runner /usr/local/libexec/mill-board-bbp-guard "$@"`) set as `MILL_BOARD_BBP_BIN`. The sudoers rule allows only that guard as `orch-runner`, never `bbp` with an argument glob: `bbp` keeps the last of a repeated flag, so a glob rule would let a second `--dir` point anywhere. The guard admits only `inbox` or `human`, requires `--dir /srv/lel/bbp` as given by `bbp_cli.rs`, refuses any further `--dir`, and execs `bbp` (script in `1092-deploy-layout.md`). The unit drops `NoNewPrivileges=true` (sudo needs setuid) and keeps `/srv/lel/bbp` in `ReadWritePaths`.
**Why.** Nano concurred that the board should not share `orch-runner` with the harness. The sudo rule keeps the master secret owner-only and every BBP write owned by `orch-runner`, and the guard limits the board to the two inbox verbs on one directory (a later `--dir` is refused, not just shadowed). `bbp_cli.rs` runs `<bin> inbox|human --dir <root> ...`, so the wrapper and guard need no code change. Review on #1763 (2026-11-17) found the first version, a glob rule on `bbp` itself, allowed a second `--dir`.
**Rejected.** a) `orch-runner` (Nano's first tap, superseded): the host-side Claude harness could read the board's tokens. Group-sharing `/srv/lel/bbp` with `master` 0640: `bbp` would accept it (`master::load` only reads), but it gives the board the key to every execution token, needs group-writable log files against the host's owner-only contract (`write_private`, 0700 dirs), and mixes file owners in the log directory. Keeping `baileyrd`: that is Nano's login, not a user of the board's own.
**Tag.** [stated] (own user) · [default] (name `mill-board`, sudo mechanism) · remind-me: mem_3551f80a018b47b7ae4c7055920fffe6
**Key.** lel:1092:board-runs-as-own-user (was lel:1092:board-runs-as-orch-runner)
**Source.** Nano, Millwright/coordinator chat `cmsg_01Y8j3ihrYW8jedySti37M9cTgXqQZPe3wekboPb1XGTKf`, 2026-11-16T19:17:15Z, verbatim "1. Concur", answering `cmsg_01Y8j3ihrYW8jedySti37M9cRTcDaiDypSf34EAPPbLppW` ("should Mill Board run as the orch-runner user? That would let the Claude harness read the board's tokens. I'd give Mill Board its own user."). Supersedes his tap on "a) orch-runner", 2026-11-16T16:15:00Z, card `cmsg_01632CBKCXsWe6RyFWvE9dovUsbPrkpu7L7hajGhXCVDa8` in thread `cmsg_01632CBKCXsWe6RyFWvE9dovKYDoFmtvXi6TAYDxoS9Gmq` (remind-me mem_056ed8754777410b9abf716a13542a80, now superseded). The user name and mechanism are the expert's defaults, 2026-11-17. Amended in place because D132 has not merged. Design in `1092-deploy-layout.md` (PR #1744). Within D130 and D131.

## D133. Packs load lazily; a role is defined by its lens; the project expert becomes a domain knowledge pack

**Decision.** Nano's direction, in his words: lens packs, skill packs and the rest are lazy-loaded. A role is defined by its lens. The project expert goes away and a project domain knowledge pack replaces it. Skill packs decide which packs load for a role: its domain pack, its lens pack, and skill packs matched to its traits, such as Rust. Ai Env Manager catalogs the packs. He gave no timing, format, location or build order, so this is direction, not a build instruction.
**Reading (the caller's and the expert's, not Nano's words).**
1. It is the design basis for #723 (role + overlay + knowledge composition). Under D127 that design waits until after the first run, and nothing is built from this yet.
2. #1123 (v0 flat lens prompts, `1123-lens-prompts.md`) does not change for M1. Each `lenses/<id>.md` is a flat file orch injects verbatim (ADR-0021), and it stays valid as a future "lens pack" once a catalog exists. orch needs no resolver for M1. The factory `roles/_protocol.md` § Loading context (protocol, role, overlay, then only listed knowledge) is a precedent for lazy loading, but its roles are workflow roles, not lenses. How the two relate is #723 design.
3. "Project expert replaced by a domain pack" does not retire `let-em-loose-expert` or the CLAUDE.md experts table now. The expert stays the answerer until a domain pack exists and Nano confirms the swap (open Q47). This is the expert's reading.
4. It extends, and does not conflict with, the 2026-11-16 morning voice decision that skill packs are lazy-loaded, with a manifest and thin loader (remind-me `mem_7634e492a3cf428b8fd9732c87deba65`, `mem_362db0e220974f16b808cf1fa20530d5`). The new part is that the lens defines the role and a domain pack replaces the expert.
5. Touchpoint for Ai Env Manager (its design belongs to its own expert): it catalogs the packs, and its manifest generation would have to say which lens, domain and trait packs a role loads. Where the pack files live is open (Q46).
**Why.** It keeps the board model (roles as lenses, D8-style critic lenses) as the one definition of a role, and replaces a hand-written expert agent with data that a loader can attach on demand.
**Rejected.** Reading it as a request to change #1123 or start #723 now (D127). Reading it as retiring the expert today.
**Tag.** [stated] for the quoted direction; items 1-5 are readings · remind-me: mem_e0d4d0e7937a4adbaeaab4a7788e5f50
**Key.** lel:general:packs-lazy-loaded-lens-defines-role
**Source.** Nano, Millwright project thread message `cmsg_01632CBKCXsWe6RyFWvE9dovG75zPmnNVZB8k4NUGqW9h4`, 2026-11-16T16:53:53Z, verbatim: "So Lens, etc, Skill packs should be Lazy-loaded. A role is defined by the lens. The project expert goes away and is replaced by a project domain knowledge pack. Skill packs determines which packs load: its domain pack, lens pack, plus skill packs matched to its traits, such as Rust. Ai Env Manager catalogs the packs." Relates to D127, D130, #723, #1123, D19 (Factory is source of truth). Numbered D133 after D132.

## D134. #1124: no paid runs for now; the first smoke run is local-only and no spend ceiling is authorised

**Decision.** Asked which spend ceiling should apply to the paid half of the smoke run (A smoke only, B smoke + 1 stage-1 run, C full M2), Nano chose "No paid runs for now". No ceiling is authorised. The options in `1124-spend-estimate.md` § 5 stay on file for when he revisits.
**Reading (the expert's, not Nano's words).**
1. #1124's done-condition "declined paid runs" is met.
2. The paid half of #1166 is deferred. The first smoke run uses local models only (Qwen3.6-35B-A3B and gpt-oss-120b via llama-swap), with no Claude or Codex lens call. "Paid" here follows the card's and D38's word: a flat-rate subscription call is held back too, until Nano names a ceiling.
3. The M2 paid experiment (#600 stage-1 runs) has no ceiling and no paid run is authorised. D38's $200 / $25 / $40 still bind Phase B only, as before; nothing carries over to M2.
4. Config is unchanged: v0 still refuses any nonzero price (ADR-0021).
**Why.** It is Nano's answer to a decision card. It also removes the one ask-first item (paid model calls) that stood between the smoke-run plan and a first run.
**Rejected.** Options A, B and C of the spend card. Treating "no paid runs" as only the dollar half and still running subscription Claude and Codex lenses.
**Tag.** [stated] · remind-me: mem_ade5caad779242259f0a7c5b11ed64d3
**Key.** lel:1124:no-paid-runs-for-now
**Source.** Nano, Millwright project thread `cmsg_01632CBKCXsWe6RyFWvE9dovKYDoFmtvXi6TAYDxoS9Gmq`, tap on decision card `cmsg_01632CBKCXsWe6RyFWvE9dovEdZwxaX7X8V2c9ew2bfAKe`, 2026-11-16T17:20:01Z, question "Which spend ceiling should apply to the paid half of the smoke run?". Confirmed in words 2026-11-16T19:17:15Z, Millwright/coordinator chat `cmsg_01Y8j3ihrYW8jedySti37M9cTgXqQZPe3wekboPb1XGTKf`, "1. Concur", answering `cmsg_01Y8j3ihrYW8jedySti37M9cRTcDaiDypSf34EAPPbLppW` ("should paid model runs stay off for now? I'd say yes"). Relates to D38, D130, D131, D127, `1124-spend-estimate.md`. Numbered D134 after D133.

## D135. #1123: Factory gets a top-level `lenses/` and `lens.json`

**Decision.** Add `lens.json` and `lenses/{architect,research,adversary,operator}.md` at the top level of `baileyrd/Factory`, matching `prompt: "lenses/<id>.md"` in `STARTER`. They do not go under `roles/`, `knowledge/`, `standards/` or `overlays/`. Factory's `README.md` layout block gets one `lenses/` line. This answer is the Approver (Nano) approval that Factory's base-change rule requires.
**Why.** D19 names Factory as the source of truth, and `lens.json` points at `lenses/<id>.md`. The layout can be undone by a PR. `tools.check` does not scan `lenses/`, so the rusty_mill fixture test is the only check on these files.
**Rejected.** b) keep the lens files only in rusty_mill for v0; c) put them under `overlays/` as a let-em-loose overlay draft.
**Tag.** [stated] · remind-me: mem_ef7ef639c5ea40e5842f0ee74cc142d6
**Key.** lel:1123:factory-top-level-lenses
**Source.** Nano, Let Em Loose project thread "Update Let Em Loose card statuses", message `cmsg_01WgyiM31RsSnZTgNuPX2VE4QN6xVQMjn5R66V99qaAV3F`, 2026-11-16T18:54:44Z, verbatim "1. Yes 2. No", answering `cmsg_01WgyiM31RsSnZTgNuPX2VE4G61A94yPvYnAq2kXZprtE5`; repeated 2026-11-17T02:19:18Z (`cmsg_01WgyiM31RsSnZTgNuPX2VE4QtLAqh3fwqpLiBmAuUesYN`, "1. Yes. 2. No,"). ESCALATE 1 of `1123-lens-prompts.md`, option a. Builds on D19.

## D136. #1123: the operator lens loads no knowledge file in v0; its prompt stays flat (D34)

**Decision.** In v0 the operator lens does not load `factory/knowledge/linux-single-host-ops-expert.md`. Its prompt stays flat, written from D34. Loading knowledge waits for composition (#723), which comes after the first run (D127).
**Why.** orch has no prompt composition, and appending the knowledge file would be new design during the D127 freeze. D133 sets the direction for lazy-loaded packs.
**Rejected.** b) append the knowledge file to the operator prompt now.
**Tag.** [stated] · remind-me: mem_69d5e68802c74fe29c21ff8c15684468
**Key.** lel:1123:operator-lens-no-knowledge-v0
**Source.** Nano, Let Em Loose project thread "Update Let Em Loose card statuses", message `cmsg_01WgyiM31RsSnZTgNuPX2VE4QN6xVQMjn5R66V99qaAV3F`, 2026-11-16T18:54:44Z, verbatim "1. Yes 2. No", answering `cmsg_01WgyiM31RsSnZTgNuPX2VE4G61A94yPvYnAq2kXZprtE5`; repeated 2026-11-17T02:19:18Z (`cmsg_01WgyiM31RsSnZTgNuPX2VE4QtLAqh3fwqpLiBmAuUesYN`, "1. Yes. 2. No,"). ESCALATE 2 of `1123-lens-prompts.md`, option a. Relates to D34, D127, D133.

## D137. `rusty_orch lens breaker` is a read-only report with no reset

**Decision.** `rusty_orch lens breaker <config> --bbp --dir --task --state --experiment` (card #758 item 2, every flag required) is read-only and has no `reset`. It prints one JSON line: the lens config's breaker (`mode`, `window_calls`, `window_micro_usd`, `min_score_move`), the task's meter calls and spend (a read-only orch-store read, restored as `lens run` restores it, so a reservation held at a crash counts), the taint table's counts per class, and whether the task is paused, with the latest `task_paused` reason and detail from BBP. Would-trips are not recomputed: they are in the runner log (S8). Resuming a paused task stays the human's `resumed(open)`, and a new `lens run` opens a new window.
**Why.** D86 and 598-build/PLAN.md make the command a read-only report. S8 keeps no breaker table and the meter stores only per-task totals, so there is no window to reset or replay; adding either would be new design under D127.
**Rejected.** A `reset` subcommand; persisting windows so past would-trips can be rebuilt.
**Tag.** [default] · remind-me: mem_f27c9299e16a4296b20b69cbac7b212c
**Key.** lel:758:lens-breaker-report
**Source.** let-em-loose-expert, 2026-11-17, for the #758 build session (PR #1764). D86; PLAN PR 3 and S8; `orch-store/src/meters.rs`.

## D138. orch maps `sensitive_paths` to launcher `hide` via agents.json, and `real_data` to `bbp dry-run`

**Decision.** Card #758 item 3, inside D62's approved Factory keys. The only path from orch to the host's filtered view is orch-bbp's `agents.json` (`materialize`, read by `bbp mod --agents` into `HarnessSandbox.hide`). So `orch_bbp::Sandbox` gains `hide` (passed through unchanged, omitted when empty), and `orch_bbp::Config` gains `sensitive_paths` (globs), copied into the `hide` of every confined pool agent. With non-empty `sensitive_paths`, load refuses a pool agent with no `sandbox` (fail closed); glob and root checks stay the host's (`confine.rs`). `real_data` (`{name: abs_path}`, names `[a-z0-9_-]+`, absolute paths) sits beside it, validated at load, and maps to `bbp dry-run --name N --data a=/p,b=/q --work DIR -- PROGRAM ARGS` (comma-separated, B4) through one thin orch pass-through that prints only bbp's summary (plan path and line count), never the plan. `lens run` is unchanged: it launches no harness and reads only through `bbp mcp`, so its data boundary is the classes and taint of item 1.
**Why.** Putting each key where it is consumed is the most reversible choice; the host already does the filtering, refusal and dry-run confinement (#1499).
**Rejected.** Keys on the lens config (nothing there launches a harness or a dry run); hiding paths in `lens run`'s MCP reads (a host change); letting an unconfined agent run with `sensitive_paths` set.
**Tag.** [default] · remind-me: mem_2c2213dcd287429e8d8c64398a3737f4
**Key.** lel:758:sensitive-paths-real-data-mapping
**Source.** let-em-loose-expert, 2026-11-17, for the #758 build session. D62; PLAN PR 3, S4, B4; `rusty_bbp_host/src/confine.rs` (`HarnessSandbox.hide`), `filtered.rs`, `dry_run.rs`, `bin/bbp.rs` (`dry-run` usage); `orch-bbp/src/lib.rs` (`materialize`), `orch-bbp/src/config.rs`.

## D139. A lens post with no taint entry reads as `sensitive` (unknown ids from lost posts)

**Decision.** Card #758 item 1, inside the approved taint rule (598-build/PLAN.md PR 3; D59, D62). Every landed lens post gets a taint entry. When a post's answer is lost twice (`PostError { unsure: true }`), its id is unknown and no entry can be written. This also happens when the run stops between a post and `save_taints`. So when a Session builds cards and lines, any message whose `from` is a lens principal and has no entry in the current taint table reads as `sensitive`. A lens principal is a configured one (lens ids and the alien id), a lens id the run's persisted state has jobs for (kept or in flight), or any author of a message the table has an entry for, since only lens posts are tainted. That keeps a lens that was later removed or renamed covered from the run's own history, independent of the current config. `from` is the session principal, which `Child::spawn` sets to the lens id through `BBP_PRINCIPAL`. The rule is checked when cards are built and never saved, so a post whose entry lands later reads at its real class. `partial` is unchanged, and the run still halts with `Halt::Board`.
**Why.** It fails closed in the same round, after a crash and on every later run, with no new orch-store record, no BBP change and no new pause reason, so it adds no new design under D127. D86 already rules out inferring an unknown post's id.
**Rejected.** Reading forward after an unsure post and tainting the lens's own new messages, because that covers only the same round. Persisting the unsure op and replaying it at the next start, because that is a new store record. Pausing the task on an unsure post, because that is a new pause reason and new behaviour. Leaving it as a documented gap, because it breaks D59's rule that sensitive data stays local. Matching on `from_role`, because it is `None` for human posts and depends on the roster.
**Tag.** [default] · remind-me: mem_4c08e98488ae4605a4afe1e41af0e4f4
**Key.** lel:758:unknown-lens-post-reads-sensitive
**Source.** let-em-loose-expert, 2026-11-17, answering a Codex P1 finding on PR #1764 (`orch-lens/src/round.rs` `partial`). 598-build/PLAN.md, D59, D62, D86, D127. Numbered D139 after D138.

## D140. One paid lens run approved for 2026-11-17 (`--max-passes 5`, default call cap); supersedes D134 for that run only

**Decision.** Nano approved ONE live paid `rusty_orch lens run` on 2026-11-17. The run is `rusty_orch lens run --max-passes 5` with the default call cap (`task_calls`, STARTER 400). This supersedes D134's "no paid runs" for that single run only. D134 stands for every other run; any further paid call, a second run included, is ask-first again. Dollar caps (`experiment_usd`, `task_usd`) do not enforce in v0, because the meter charges 0 per call (ADR-0021). The "$25" the user stated is a report threshold only. "Board directory" means the BBP directory `/srv/lel/bbp` (user-confirmed).
**Why.** Nano's own words in the saved chat: after "Lens runner declared done 2026-11-05; a live paid run needed your approval and none is recorded. Make it today?" and the sequence "dry-run, live run capped at five passes, read the board", he answered "Yes, let's do it now." The goal is to see models use the BBP board and find what breaks (`mem_f2c87231f0fa4845952dad1fa237712e`).
**Rejected.** Reading it as a ceiling for later runs. Reading it as a dollar cap (nothing enforces one). Treating the subscription Claude lens as cleared: `/etc/let-em-loose/assertions.json` has `no_overage` for codex only (checked 2026-11-03, lapses about 2026-12-03) and none for claude; Q50 asks Nano which providers may be paid.
**Caveats.** The `--max-passes 5` and call-cap limits come from the chat sequence and the user's relay, not from Nano's "Yes" alone. The run intent (the rusty_mux SSH host launcher brief) is the user's relay; the chat has Claude suggesting it and Nano sending Orca screenshots, but Nano has not confirmed the brief as the task. The run is still blocked on #1092 (deploy: no `bbp`, `/opt/lel`, `/srv/lel/bbp`) and #1123 (no real `lens.json`); ask-first gates for deployment still apply.
**Tag.** [stated] · remind-me: mem_76131c8f54154cbd8e93a0edbb4cd4d7
**Key.** lel:1124:one-paid-lens-run-2026-11-17
**Source.** Nano, claude.ai chat 2026-11-17, remind-me capture `cap_675c3da34b0946cdb0900aec848a07ba` (dialog `mem_307a8af289f84ce1b918249ea0691ab6`), noted first in `mem_f2c87231f0fa4845952dad1fa237712e`; relayed by the user (baileyrd) in Claude Code the same day, who gave these references. Relates to D134, D38, ADR-0021, `1124-spend-estimate.md`. Numbered D140 after D139.

## D141. Run 1 toy `sweep` smoke run stays local-only; run 2 SSH host launcher brief is paid (codex now, claude after its `no_overage` entry)

**Decision.** Nano answered Q50 himself, then corrected the record the same day (see Amended). Run 1 is the #1166 smoke run on the toy `sweep` intent, **local-only**, under that design's limits: `lens run --max-passes 8`, `task_calls 60`, `experiment_usd`/`task_usd` 1 at $0 real cost, providers `local`, `local-high`, `local-rebel` only, no paid provider (`1166-smoke-run.md` sections 3 to 7). Run 2 is the SSH host launcher brief for rusty_mux (Orca Remote Hosts panel), after run 1 has finished and been read. Run 2 is paid, with the D140 limits: `rusty_orch lens run --max-passes 5`, the default `task_calls` cap (STARTER 400), and no dollar-cap enforcement in v0 (ADR-0021; "$25" is a report threshold only). Paid providers for run 2: codex now; claude only once its `no_overage` entry exists in `/etc/let-em-loose/assertions.json` (today codex only, checked 2026-11-03, lapsing about 2026-12-03). This supersedes D140's "any second run is ask-first" for run 2, which needs no separate go. D140's single paid run is therefore run 2. A third run, a higher cap or an unlisted provider stays ask-first.
**Why.** Nano's own words: "1. The smoke run uses the toy sweep intent as designed. The SSH host launcher brief is the second run. I approve both paid runs, same limits each. 2. Paid providers: codex now; claude once I add its no_overage entry." This closes Q50's two open asks.
**Rejected.** Treating the launcher brief as the first run, since #1166 is designed around `sweep`. Paying the claude lens before its assertion exists. Reading "same limits each" as a shared budget across both runs. Giving run 1 the paid limits (corrected, see Amended).
**Notes.** D134 still holds for everything else. Both runs wait on #1092 (deploy: `bbp`, `/opt/lel`, `/srv/lel/bbp`; Q-answers for the deploy). The lens config (#1123) landed in baileyrd/Factory#1 on 2026-11-16 and still has to be on the host. Claude lenses additionally wait on the assertion; until then a run with the claude lens is not paid-approved. Local models (llama-swap `<llama-swap-host>:<port>`, model id `qwen3.6-35b-a3b`) cover any lens not paid. Dated, not a ceiling: a second run only after the first has finished and been read.
**Amended 2026-11-17.** First written as "both runs paid with `--max-passes 5` and call cap 400". Nano then said, typed in the session and saved verbatim as `mem_c5696762bdfb44f6aa7a169beceee027` (speaker Nano): "D141 says both runs are paid with --max-passes 5 and call cap 400, but run 1 is local-only at --max-passes 8 and task_calls 60. Ask the expert to amend D141: run 1 is local-only under the #1166 design's limits; the paid limits apply to run 2 only. This is a correction to the record, in Nano's name." Amended in place, not as a new D number: the correction is the same day, changes no other entry and keeps the key. The first wording of his Q50 reply ("I approve both paid runs, same limits each") is kept in the Source for the record.
**Tag.** [stated] · remind-me: mem_2357f42a0f9c42ea8a1b7b4fddf537db
**Key.** lel:1124:two-paid-runs-codex-first
**Source.** Nano, typed into a Claude Code session on BaileyAi, 2026-11-17, saved verbatim by the coordinator as remind-me `mem_efab3121ffc64728a2ee6ba8ec5a5f9d` (speaker Nano). Builds on D140 (`lel:1124:one-paid-lens-run-2026-11-17`), D134, ADR-0021, `1166-smoke-run.md`. Amendment: remind-me `mem_c5696762bdfb44f6aa7a169beceee027`. Numbered D141 after D140.

## D142. #1092: `/srv/lel` paths stay `container_file_t` on BaileyAi; no relabel before run 1

**Decision.** `/srv/lel/bbp`, `/srv/lel/orch-state`, `/srv/lel/lenses` and `/srv/lel/lens.json` keep the SELinux type `container_file_t`. Nothing is relabelled before run 1 (#1166). The `var_t` that the deploy prompt told the operator to expect was wrong. On BaileyAi `matchpathcon` returns `container_file_t` as the policy default for these paths (a local fcontext rule on `/srv/lel`), so the labels are as intended. `restorecon -Rv` printed "not reset as customized by admin" because `container_file_t` is a customizable type, and `restorecon -F` would change nothing.
**Why.** Run 1 and `lens check` run as `orch-runner` started with `sudo -u orch-runner` from `baileyrd`'s shell, which is `unconfined_t`; llama-swap is `unconfined_t` too, and `unconfined_t` reads `container_file_t`. `lel-codex` mounts only `/srv/lel/machine-id`, `/srv/lel/experiment` and a volume, so no confined service touches these paths yet. Facts checked on the host by the coordinator, 2026-11-17.
**Revisit when.** (a) The `mill-board` systemd service (D132) is deployed: its wrapper runs `bbp` as `orch-runner` through sudo, so check that `/srv/lel/bbp` is readable and writable in the resulting domain. (b) Run 1 shows an AVC denial (`ausearch -m avc`). The `bbp open` sandbox probe (user namespaces or SELinux) is the first place a label problem would show. The fix then is a `semanage fcontext -a -t <type> '/srv/lel/bbp(/.*)?'` rule followed by `restorecon -Rv /srv/lel/bbp`, not `restorecon` alone (the policy default is `container_file_t`, so it is a no-op). Never run `restorecon` on `/srv/lel` or `/srv/lel/experiment` (the `lel-codex` bind mounts). The deploy and smoke-run prompts (`1092-deploy-prompt-bbp-and-board-dir.md`, `1166-smoke-run-prompt.md`) still expect `var_t` and should be corrected.
**Rejected.** Relabelling to `var_t` now: a local policy change with no reader that needs it, and `restorecon` could not do it anyway. Running `restorecon -F`: no effect.
**Tag.** [default] · remind-me: mem_f02b0b40230741e89e0a16de2496d4dd
**Key.** lel:1092:srv-lel-labels-container-file-t
**Source.** let-em-loose-expert, 2026-11-17, answering the coordinator for Nano. D131 (deploy on BaileyAi), D132 (`mill-board` service, not yet deployed), `1092-deploy-layout.md`, `1092-deploy-prompt-bbp-and-board-dir.md`. Numbered D142 after D141.

## D143. D127's "first run" is a clean #1166 pass; the freeze holds until a rerun meets it

**Decision.** Asked what counts as D127's undefined "first run", Nano chose "Clean #1166 pass first (Recommended)": the first run is a #1166 run with a settled claim, the inbox firing and `bbp verify` clean. The design freeze (D127) holds until a rerun, after the bug fixes, meets that. Run 1 (task `smoke-1`, local-only toy `sweep`, 2026-11-17) does not count: the machinery ran end to end, but criteria 2, 3 and 4 of `1166-smoke-run.md` section 9 were unmet and it halted on a refused post. D127's text is unchanged; only its meaning is fixed.
**Why.** It lifts the freeze on evidence that the loop works, not on the loop merely running. Run 1's result is in remind-me `mem_ddc10f27ebb84572b6bcec43e05d21dc`.
**Rejected.** a) Run 1 counts as the first run. c) The first run is the first live paid run (run 2, D141).
**Tag.** [stated] · remind-me: mem_7a440c37500b439f8b5239db9d97aaac
**Key.** lel:127:first-run-is-a-clean-1166-pass
**Source.** Nano, answer dialog in the Claude Code session on BaileyAi, 2026-11-17, saved verbatim as `mem_c79bd23503c349ee84fc535c423a07f8` (speaker Nano). Amends the meaning of D127; relates to D141, `1166-smoke-run.md`. Numbered D143 after D142.

## D144. #1166: rejecting `executed` from lens replies at `post::parse` is a bug fix, not new design

**Decision.** In orch-lens `post::parse`, an `evidence_kind: executed` reply from a lens is rejected at the source as a permanent provider failure for that reply, non-halting. This is a bug fix under D127. The host already refuses such a post (`check_executed`, `1166-smoke-run.md` section 10), the 1123 prompts forbid it, and orch should not post it. Run 1's local-high reply posted one, `bbp` refused it (`EvidenceRequired`) and orch halted.
**Why.** It brings orch in line with the approved design (a lens cannot post executed evidence that counts). It adds no behaviour. Changing the halt rule for board-refused posts is a separate policy question (Q51).
**Rejected.** Treating it as new design that waits for the freeze to lift.
**Tag.** [default] · remind-me: mem_164afed2da774d07bc291781207dcc53
**Key.** lel:1166:reject-executed-at-parse
**Source.** let-em-loose-expert, 2026-11-17. `1166-smoke-run.md` section 10, D127, D139, D143, `mem_ddc10f27ebb84572b6bcec43e05d21dc`. Numbered D144 after D143.

## D145. #1166: the llama.cpp 500 on valid harmony output is a host/adapter issue, not design

**Decision.** `gpt-oss-120b` via llama-swap returned HTTP 500 twice because llama.cpp (build f74e2fa, `common_chat_peg_parse`) rejects valid harmony output (`<|channel|>final <|constrain|>JSON<|message|>{...}`). Fixing it (a newer llama.cpp, a changed reasoning-format setting or a hardened `orch-openai` adapter) is a host or adapter fix and changes no lens, claim or board behaviour, so it is not new design under D127. Which fix to use is the builder's choice. **Held by D147:** the fix is not applied yet. Root cause: build f74e2fa's gpt-oss parser (`common/parsers/gpt-oss.cpp`) accepts `<|constrain|>` after `final` only when a `response_format` is set; upstream master (a9d43ec) makes it optional, so the fix is a llama.cpp rebuild.
**Why.** A provider's server bug on well-formed output.
**Rejected.** Dropping `local-high` from the roster to avoid it; that would change the approved lens roster.
**Tag.** [default] · remind-me: mem_331b473b9194401fb93c4237411fe5b1
**Key.** lel:1166:llamacpp-harmony-500-is-host-adapter
**Source.** let-em-loose-expert, 2026-11-17. `mem_ddc10f27ebb84572b6bcec43e05d21dc`, D127. Numbered D145 after D144.

## D146. #1166: an inbox-exercising seed is a test-fixture choice; thresholds and escalation rules unchanged

**Decision.** The rerun's seed may be chosen so that the inbox fires (for example a claim A that reaches the strict line, not one dropped by counter evidence). That is a test-fixture choice within `1166-smoke-run.md` section 2. It must not change the thresholds (accept 8, strict 12, `drop_at -8`) or the escalation rules.
**Why.** The seed is test input; the thresholds and rules are approved design under the freeze.
**Rejected.** Lowering a threshold or adding an escalation path so the inbox fires.
**Tag.** [default] · remind-me: mem_981c7beee6ad4b0190795bf1830e85b7
**Key.** lel:1166:inbox-seed-is-fixture-choice
**Source.** let-em-loose-expert, 2026-11-17. `1166-smoke-run.md` sections 2 and 9, D127, D143. Numbered D146 after D145.

## D147. #1166: the llama.cpp rebuild and the Qwen reasoning budget are held until the rerun's per-pass log shows which slot failed

**Decision.** Nano: "Hold both host changes (llama.cpp rebuild, reasoning budget) until the rerun's per-pass log shows which slot failed and why. One change at a time." So D145's fix (rebuild llama.cpp past build f74e2fa) and a Qwen reasoning budget (`--reasoning-budget 2048` in `~/llama/config.yaml`, untested) are held and not applied. The rerun is task `smoke-2` (`1166-smoke-rerun.sh`) on the code fixes D144 and D146 plus the stderr per-pass log (rusty_orch PR #1783). Host changes, one at a time, only after that log names the failed slot. When a rebuild is applied it must overwrite the shared libraries (not `cp -n`, which would keep the old `.so` files) and keep the rollback copy `~/llama/bin/vulkan.f74e2fa`.
**Why.** Run 1 had only 2 of 3 candidates and which slot failed and why is unknown, because the per-pass log was lost to the closed-stdout panic (fixed in #1783). Changing the host first would hide the cause.
**Rejected.** Applying the rebuild and the reasoning budget together before the rerun.
**Tag.** [stated] · remind-me: mem_157ec68e43034d219af811c9735b1681
**Key.** lel:1166:hold-host-changes-until-rerun-log
**Condition met 2026-11-17 (smoke-3):** the per-pass log now names the failed slots (see `1166-smoke-run.md` Run 3 result). The hold stays: no host change is applied (D152).
**Source.** Nano, typed in the Claude Code session on BaileyAi, 2026-11-17, saved verbatim as `mem_7c2db8a737f0414898ce9e85a045f46e` (speaker Nano). Relates to D143, D144, D145, D146, Q52. Numbered D147 after D146.

## D148. #1166 smoke-2: candidates never reviewed (stale question view) is an orch bug fix, re-read the question with `claim_card`

**Decision.** A lens seat keeps each claim view from `claims_delta` and refreshes only the claims the delta returns. `delta_ids` (rusty_bbp `claims.rs`) returns claims whose status, score or evidence changed, plus dependents (addendum, claims_delta), and a new candidate that `answers` Q does not mark Q changed. So within one `lens run` process Q's cached view keeps `candidates: []`, `attack_ready` (orch-lens `schedule.rs`, needs at least 2 candidates) never fires, and the candidates are never attacked, so Q stays open. Run 1 had the same cause: candidates were reviewed only in the resume process, which re-read from rev 0. This is a **bug fix in orch, not new design**: when a delta carries a claim whose `answers` is Q, the seat re-reads Q with `claim_card`. The protocol-side fix (make `delta_ids` include the answered question) changes an accepted `claims_delta` contract in the BBP core, an ask-first public API change, and is not chosen.
**Why.** The approved design is that candidates are attacked (#594, C-R7); the runner contradicts it. The orch-side re-read uses an existing tool and changes no accepted contract.
**Rejected.** Changing `delta_ids` in rusty_bbp; restarting the process between passes.
**Tag.** [default] · remind-me: mem_c362c5cc7ba44a9e86ecd089f1dd1732
**Key.** lel:1166:candidates-never-reviewed-orch-reread
**Source.** let-em-loose-expert, 2026-11-17, from the coordinator's smoke-2 evidence (code read and a throwaway-board test). `bbp-claims-addendum.md` (claims_delta), D127, D143, Q52. Numbered D148 after D147.

## D149. #1166 smoke-2: the architect prompt must say a claim has no `evidence` field (use `refs`); a prompt bug fix

**Decision.** In smoke-2 `local-high` (gpt-oss-120b) failed Propose with `claim has unknown field "evidence"` (Permanent; the protocol allows only `refs` on a claim and parse accepts no other fields). Pass 2 skipped the slot (fallbacks already used), so only 2 candidates stood. The smallest fix is a **bug fix in the architect lens prompt** (Factory `lenses/architect.md`): a claim has no `evidence` field; cite sources in `refs`. It is prompt-only and probabilistic. Not done, because each is design: (b) `parse` accepting `evidence` as an alias for `refs` (loosens a deliberate strictness); (c) a same-provider retry for ordinary proposal slots (a scheduler change, as the rebel already has `REBEL_TRIES=2`); (d) a fourth provider id (an unused fallback, but not three distinct models). If the prompt fix fails on the next rerun, (c) goes to Nano as an escalation.
**Why.** The prompt names a shape the protocol rejects; correcting it adds no behaviour and is reversible.
**Rejected.** (b), (c), (d) above.
**Tag.** [default] · remind-me: mem_ace006f513ab4737b5553b05f9ec4df5
**Key.** lel:1166:architect-prompt-no-evidence-field
**Source.** let-em-loose-expert, 2026-11-17, from the coordinator's smoke-2 evidence. D34, D127, D135, `1123-lens-prompts.md`. Numbered D149 after D148.

## D150. #1166 smoke-3: the architect prompt also says one claim post per reply, no evidence post; a prompt bug fix

**Decision.** In smoke-3 `local-high`'s Propose(1) failed with `post 1: evidence is not allowed here`: the reply held a claim post and a second, evidence post. The architect proposal job allows only claims, so orch refused the whole reply, including the valid claim in post 0. This extends D149 (smoke-2 was an `evidence` field on a claim; this is an evidence post beside it: the same habit of attaching evidence, a different surface). The smallest fix is a **bug fix in the architect prompt** (Factory `lenses/architect.md`): "Reply with one claim post and no evidence post." Not done, because it is design: `parse` dropping posts the job may not make and keeping the allowed ones (it loosens the deliberate refuse-whole-reply rule). One run cannot say whether D149 shifted the form or it is chance. Nano: "Go on D150 as a Factory PR" (D151).
**Why.** A prompt sentence adds no behaviour and is reversible; the pass-2 fallback already recovered the slot.
**Rejected.** The parse change above.
**Tag.** [default] · remind-me: mem_016934faf1a64c829986b883930c35e6
**Key.** lel:1166:architect-prompt-one-claim-no-evidence-post
**Source.** let-em-loose-expert, 2026-11-17, from the coordinator's smoke-3 evidence. D149, D127. Numbered D150 after D149.

## D151. #1166: the first-run bar is met through reviewer prompt wording (support only when the brief supports the claim); Q54 resolved for now

**Decision.** Asked how the first-run bar is met when the reviewers only attack, Nano chose option a: "Change the reviewer prompts so they also post support when a claim holds. Add one condition: support only when the reviewer can point to text in the brief that the claim satisfies, and post nothing when unsure." So the route to D143's clean pass is prompt wording, not a rule. Q54 is resolved for now. A BBP rule that escalates a question whose candidates all dropped stays unrequested and returns to Nano only if this fails on the rerun. This is not new design under D127; Nano approved the wording.
**Implementation (Nano: "Go on D150 as a Factory PR. Put the reviewer wording in the same PR").** One Factory PR holding: D150, in the architect lens: "Reply with one claim post and no evidence post."; and in `lenses/adversary.md` and `lenses/operator.md`: "When a claim holds, say so: post support, but only if you can point to text in the brief that the claim satisfies. Cite the brief as `art:1` and quote that text in the note. If you are not sure, post nothing." The adversary's earlier "Support a claim only with a cited reason an attack failed" is replaced. Factory#3 is on main; the last deploy cloned Factory at 831268c.
**Finding.** orch never shows a lens the brief's artifact id, so the wording hard-codes `art:1`. That holds for every task so far, because the brief is the task's first artifact; a revised brief would get another id. Whether orch should render the brief's id in the prefix is a small code change: Q55.
**Why.** Smoke-3: four of four claims countered, no support ever posted, so nothing could reach accept. Prompt wording adds no behaviour and is reversible, and the "only with text in the brief, nothing when unsure" condition limits the risk of lenses supporting claims on opinion.
**Rejected.** b) a BBP rule escalating a question whose candidates all dropped (BBP change, ask-first, new design); c) accepting the criterion as it stands.
**Tag.** [stated] · remind-me: mem_bf4922601cd94e8ab11e614828d80c09
**Key.** lel:1166:reviewer-support-wording-first-run-route
**Source.** Nano, typed in the Claude Code session on BaileyAi, 2026-11-17, saved verbatim as `mem_176bc02b805a480b9806b8e3ee0945a3` (speaker Nano), call 1. Amends the route of D143 and D127; relates to D150, Q54, Q55. Numbered D151 after D150.

## D152. #1166: no host change; rerun smoke-4 after the prompt fixes alone (D147 hold stays)

**Decision.** Nano, call 2 of the same answer: option b, "No host change. Rerun after the prompt fixes alone." D147's hold stays: the llama.cpp rebuild (D145) and the Qwen reasoning budget are not applied. The rerun is smoke-4 after the prompt fixes alone: D148 (the orch re-read, done), and D149, D150 and the reviewer wording of D151, all in the Factory PR. If the empty replies at the 4096-token cap recur, that is the evidence for the reasoning budget, one change at a time.
**Condition recurred 2026-11-17 (smoke-4):** three empty Qwen replies again, and they cost the third candidate (run 4 result). Applying any host change is still Nano's call (not yet answered).
**Why.** One change at a time: the prompt fixes get measured alone before any host change.
**Rejected.** a) applying the Qwen reasoning budget first; c) also rebuilding llama.cpp (no new evidence in smoke-3).
**Tag.** [stated] · remind-me: mem_4fc7c5f64a84458a9fbdf43dc3fd84ce
**Key.** lel:1166:no-host-change-rerun-smoke-4
**Source.** Nano, typed in the Claude Code session on BaileyAi, 2026-11-17, saved verbatim as `mem_176bc02b805a480b9806b8e3ee0945a3` (speaker Nano), call 2. Confirms D147's hold. Numbered D152 after D151.

## D153. #1166: the Qwen reasoning budget (`--reasoning-budget 2048`) is the one host change allowed before smoke-5; the llama.cpp rebuild stays held

**Decision.** After smoke-4 Nano chose option a, "Qwen reasoning budget only": add `--reasoning-budget 2048` to the `qwen3.6-35b-a3b` entry in `~/llama/config.yaml` (untested), then rerun as smoke-5. This lifts D152's no-host-change hold for exactly that one change. D147's one-change-at-a-time stays, and the llama.cpp rebuild (D145) stays held; the gpt-oss HTTP 500s cost only absorbed calls. Smoke-5 is judged on the same section 9 criteria.
**Correction 2026-11-18 (smoke-6):** the earlier "effective: 0 empty Qwen replies" claim is withdrawn. In smoke-6 there were 7 empty Qwen replies and seven Qwen generations of exactly 4096 tokens in the llama-swap log. The flag was never active in smoke-5 or smoke-6: llama-swap (started 2026-11-16 19:01:54) was never restarted after `~/llama/config.yaml` was edited (mtime 2026-11-18 08:36:58), it keeps its config in memory, and the running Qwen llama-server (started 2026-11-18 10:38) has no `--reasoning-budget` in its arguments. So the budget is applied in the file but not loaded; Nano must run `systemctl --user restart llama-swap`. Smoke-5's 0 empties was chance (few Qwen calls), not proof. The per-request test (3 of 3 empty without, 6 of 6 valid with `reasoning_budget_tokens` 2048) stands. The llama.cpp rebuild (D145) stays held. Drift: a coordinator memory (`mem_e66abb6f86404568ac8605aa7ff51f79`) says the change was applied "with a llama-swap restart"; the process start times say otherwise.
**Why.** Empty Qwen replies (`reply is not JSON: unexpected end of input`) recurred in smoke-3 (3) and smoke-4 (3) and cost the third candidate in smoke-4; the llama-swap log shows four Qwen generations of exactly 4096 tokens, orch's output cap. That is strong evidence, not proof, that thinking consumes the cap.
**Note.** llama-server accepts the budget per request (`reasoning_budget_tokens` in the request body, `tools/server/server-common.cpp`), so the effect can be tested with free local calls before the config is edited (the coordinator is running that test). Claim 6 in Nano's inbox (smoke-4) is his own call: `bbp human ... claim 6 accept`, or `choose 1 6`.
**Rejected.** b) no host change; c) rebuilding llama.cpp first.
**Tag.** [stated] · remind-me: mem_674628bd3ea1445480debdb36683290f
**Key.** lel:1166:qwen-reasoning-budget-then-smoke-5
**Source.** Nano, answer dialog in the Claude Code session on BaileyAi, 2026-11-17, saved verbatim as `mem_1c343b7650f94eb3bf7cf89344acb380` (speaker Nano). Narrows D152; confirms D147's one-at-a-time rule. Numbered D153 after D152.

## D154. #1166 smoke-4: Nano drops claim 6 (grounded score 0, deviates from the brief); Q has no accepted or chosen candidate

**Decision.** Nano: "I am dropping claim 6 on smoke-4: grounded score 0 and it deviates from the brief." Claim 6 (`local-high`, escalated `irreversible` at board score 14 against the strict line 12) is dropped by his own human action. Q on smoke-4 therefore has no accepted or chosen candidate, so section 9.3 stays unmet by this run (D143).
**Why.** The grounding audit (see Q53 and `1166-smoke-run.md` Run 4 result): of the five votes only msg 10 and msg 14 rest on the brief (grounded score 0, or +4 without msg 14), and only one grounded support author (the adversary) against the two the accept line needs. The claim also deviates from brief line 5 (Q58).
**Rejected.** `claim 6 accept`, `choose 1 6`.
**Tag.** [stated] · remind-me: mem_ca29e164a4d5443e932187d5dc258b10
**Key.** lel:1166:drop-claim-6-smoke-4
**Source.** Nano, typed in the Claude Code session on BaileyAi, 2026-11-18, saved verbatim as `mem_3bb04ac898ca4e4eb0fdb8f2d3c61739` (speaker Nano). Relates to D143, D151, Q53, Q58. Numbered D154 after D153.

## D155. Standing rule: every run's read-back includes the grounding audit before Nano acts on an escalated claim

**Decision.** Nano: "From now on, produce this same grounding audit in the read-back of every run, before I act on any escalated claim." For each vote on an escalated claim the audit quotes the exact brief sentence the note relies on, or marks the vote ungrounded, and checks specifically whether the brief itself says the technical wording the note attributes to it. It gives the score three ways: all votes (the board's), grounded only (with repeats), and one vote per author (latest and first). The audit judgement stays with Claude; the coordinator's script collects the evidence bundle (brief text, the claim, each vote with its note and citation).
**Why.** In smoke-4 the board's 14 counted votes that repeat the claim's own wording, cite a claim as a run that never happened, or attribute wording to a brief that does not contain it (Q53). Nano wants to act on what the brief supports.
**Rejected.** Acting on the board score alone.
**Tag.** [stated] · remind-me: mem_4c73b1961a914fd985493a4652e4b18e
**Key.** lel:1166:grounding-audit-in-every-readback
**Source.** Nano, typed in the Claude Code session on BaileyAi, 2026-11-18, saved verbatim as `mem_3bb04ac898ca4e4eb0fdb8f2d3c61739` (speaker Nano). Relates to D154, Q53. Numbered D155 after D154.

## D156. #1166 smoke-5: the architect prompt must say every claim sets `answers` to the question's id; a prompt bug fix

**Decision.** In smoke-5 `local`'s Propose(0) failed Permanent with `post 0: a proposal must answer msg:1` (the claim had no `answers`), and pass 2 skipped the slot (`no cleared provider`), so again 2 candidates from 2 providers, the loss now by a parse refusal and not an empty reply. It is the third form of an architect reply being refused, after smoke-2 (`evidence` field, D149) and smoke-3 (`evidence` post, D150). The smallest fix is a **bug fix in the architect prompt** (Factory `lenses/architect.md`): set `answers` to the question's message id on every claim. Not done, because it is design: orch filling `answers` itself or relaxing the check. If a fourth refusal form appears, the pattern goes to Nano.
**Why.** The protocol requires `answers` on a proposal and the prompt omits it; adding a sentence changes no behaviour and is reversible.
**Rejected.** Orch defaulting `answers`; relaxing the check.
**Tag.** [default] · remind-me: mem_4d7f8d98e2a443aebeac21dab81d31c2
**Key.** lel:1166:architect-prompt-answers-field
**Source.** let-em-loose-expert, 2026-11-18, from the coordinator's smoke-5 evidence. D149, D150, D127. Numbered D156 after D155.

## D157. #1166: keep the D143 bar (criterion 3 not reworded); build D156 and rerun as smoke-6; the all-dropped BBP rule returns to Nano only if candidates keep dropping

**Decision.** After smoke-5, where both candidates were dropped and Q stayed Open (the drops were later audited: claim 6 on grounded votes, claim 7 on ungrounded ones, see the correction below), Nano chose option a, "Keep the bar, fix and rerun": D143 stays as written (the first run is a clean #1166 pass with a settled Q). Criterion 3 of `1166-smoke-run.md` section 9 is NOT reworded. D156 (architect prompt: set `answers` to the question's id on every claim) is being built as a Factory prompt line, so three candidates stand and one may survive. The rerun is smoke-6. Only if candidates keep dropping legitimately after that does the BBP rule for an all-dropped question (D151 option b; ask-first) go back to Nano. This sets the meaning of D143 for now.
**Why.** The two losses so far are prompt-fixable, and three candidates give a survivor a real chance.
**Correction 2026-11-18.** The framing "both candidates legitimately dropped" was wrong. Nano's `bbp why --claim 1,6,7` and the D155 audit show claim 7 was dropped by ungrounded votes and would have stayed Open at a grounded -4, so the bar is reachable (Q59). Nano's choice stands (keep the bar, fix, rerun as smoke-6).
**Rejected.** b) rewording criterion 3 so that a legitimately all-dropped Q counts; c) requesting the BBP rule now.
**Tag.** [stated] · remind-me: mem_708b09f2022e4622b7d060153b9ed72a
**Key.** lel:1166:keep-first-run-bar-fix-and-rerun-smoke-6
**Source.** Nano, answer dialog in the Claude Code session on BaileyAi, 2026-11-18, saved verbatim as `mem_96800eda3fae4f51b07672e908a68ccc` (speaker Nano). Confirms D143; builds on D151, D156. Numbered D157 after D156.

## D158. The grounding audit (D155) also covers candidates the board dropped, as a read-only report

**Decision.** Nano, answering Q59: "Q59: yes. Extend 1166-grounding.sh to audit candidates the board dropped, not only escalated claims. Put it in its own PR." D155 is extended: every run's read-back audits escalated AND dropped claims, with the same method (quote the brief sentence each vote relies on or mark it ungrounded; the score three ways), as a read-only report. Q59 is closed.
**Why.** A drop on ungrounded votes decides whether Q has a survivor: on smoke-5, claim 7 was dropped by ungrounded votes (msgs 10 and 14) and would have stayed Open at a grounded -4, above the -8 drop line.
**Rejected.** Auditing escalated claims only.
**Tag.** [stated] · remind-me: mem_e8160b3d28624f2ca91300fbceb8eac2
**Key.** lel:1166:grounding-audit-covers-dropped-claims
**Source.** Nano, typed in the Claude Code session on BaileyAi, 2026-11-18, saved as `mem_928dc8aca99743ffb756ee87a6e1b0ff` (speaker Nano). Extends D155; relates to Q53, Q59. Numbered D158 after D157.

## D159. D143 criterion 3 for an audit-rescued candidate: one reopen, a lens run, then Nano's `choose` counts (D158 stays)

**Decision.** Nano: "b, with a limit. A human choose of an audit-rescued candidate counts toward criterion 3 only if a lens run after `reopen` leaves the candidate Open or accepted. One reopen per candidate. If the lenses drop it again on votes the audit shows are ungrounded, then (a) applies: I choose it, and the audit is recorded as the reason in the run record." This sets how D143's criterion 3 is read for a candidate dropped on ungrounded votes. D158 stays.
**Procedure.** (1) The read-back's grounding audit shows a candidate was dropped on ungrounded votes. (2) Nano reopens it, once per candidate (`bbp human ... claim N reopen`); reopen is refused if a dep is closed, and if the claim's budget is exhausted `budget_extended` must be recorded first. (3) A lens run on the same task (the resume script) reviews it afresh in the new epoch. (4) If the candidate is Open or accepted afterwards, Nano's `choose` counts toward criterion 3. (5) If the lenses drop it again on votes the audit shows ungrounded, Nano's `choose` counts anyway, with the audit recorded as the reason in the run record. (6) If it is dropped again on grounded votes, it does not count, and the bar is not met by that candidate.
**Where the reason is recorded.** `bbp human` has no reason or note field that Nano knows of, so "recorded as the reason in the run record" means the run record: `1166-smoke-run.md` (the Run N result) and the grounding audit file, not a board field.
**Why.** A human override counts only if the lens loop confirms the candidate, or drops it again on votes the audit shows ungrounded; one reopen per candidate bounds it. D143's "clean pass" keeps its meaning that the lens loop works.
**Rejected.** a) counting any human choose; c) never counting a human choose (a lens-driven survivor only).
**Tag.** [stated] · remind-me: mem_49f619c6e57a417daf2e0ec6e80490e0
**Key.** lel:1166:audit-rescued-candidate-reopen-then-choose
**Source.** Nano, typed in the Claude Code session on BaileyAi, 2026-11-18, saved verbatim as `mem_af1925dad3d74a6c9d84f8f67b19bfc3` (speaker Nano). Refines D143 and D158; relates to C-R8 in the claims addendum. Numbered D159 after D158.

## D160. #1571: Nano approves the agent launch profiles design as written (composition, env delivery, no service credentials in the jail, hooks deferred, build order)

**Decision.** The design in `1571-launch-profiles-design.md` (PR #1802) is approved as written: an agent file (trust cap) and a project file (needs) compose purely into one C75 `Launch`; credentials are minted per launch, delivered in the exact env and revoked by each service; the jail gets no service credentials (D101); hooks are deferred; build order as its section 6. Question 3: "Approve the design". Nano: "3. Yes." This satisfies D25 for #1571. Ask-first items inside it (the BaileyAi host writes for `agent_user`, dependency bumps, public API changes) still need their own approval.
**Why.** Profiles as data keep the later fold into the mux server a move, not a rewrite (`mem_b563d1346d6545ae9bb06a10707cf777`); the mux grants, each service revokes, orch holds both keys.
**Rejected.** None offered.
**Tag.** [stated] · remind-me: mem_556bfcda61f64e6aaa6cd38c230ca0f5
**Key.** lel:1571:launch-profiles-design-approved
**Source.** Nano, Let Em Loose project chat, message `cmsg_01WgyiM31RsSnZTgNuPX2VE4YGppB9nnPb7JnUCwC3s2Yq`, 2026-11-18T17:43:36Z: "1. Concur. 2. Concur. 3. Yes. 4. accept", answering in order the questions in thread message `cmsg_01WgyiM31RsSnZTgNuPX2VE42brap92o45tKyeDpvs4Be3` (~17:30Z). Item 4 answers #1563 and is recorded by its own thread, not here. Numbered D160, after D158-D159 (#1797).

## D161. #1571 v0 isolation is `same_user` (agents run as Nano's user); `agent_user` once the BaileyAi host writes are approved

**Decision.** v0 launch profiles run agents as Nano's own user (`same_user`). `agent_user` (`orch-runner`) follows once the four BaileyAi host writes are approved; the jail comes later. This departs from the card's "start with a dedicated user". Question 1: "v0 isolation: same_user now, agent_user once the host writes are approved". Nano: "1. Concur."
**Why.** Agents sharing orch's uid can read the minting keys under `same_user` and `agent_user` alike until the jail exists, so a dedicated user now buys little and needs host writes; no worse than today.
**Rejected.** b) `agent_user` now with the host writes approved; c) a separate uid holding the minting keys and handing orch only minted tokens.
**Tag.** [stated] · remind-me: mem_3c3a28e44b9d4dc39ef02413c70ac818
**Key.** lel:1571:v0-isolation-same-user
**Source.** Nano, Let Em Loose project chat, message `cmsg_01WgyiM31RsSnZTgNuPX2VE4YGppB9nnPb7JnUCwC3s2Yq`, 2026-11-18T17:43:36Z: "1. Concur. 2. Concur. 3. Yes. 4. accept", answering in order the questions in thread message `cmsg_01WgyiM31RsSnZTgNuPX2VE42brap92o45tKyeDpvs4Be3` (~17:30Z). Item 4 answers #1563 and is recorded by its own thread, not here. Relates to D101, D104 and `mem_931824b0f5b042618d54ed84b2d3da8e` (no sandbox dependency). Numbered D161 after D160.

## D162. #1571 profile files live in Factory (`factory/agent-profiles/*.json`, `factory/project-profiles/*.json`)

**Decision.** For v0, agent profiles are `factory/agent-profiles/*.json` and project profiles are `factory/project-profiles/*.json`, in Factory. Question 2: "Profile files live in Factory". Nano: "2. Concur."
**Why.** Orch already reads Factory, and `resolve` does not care where files come from, so the location is cheap to move later.
**Rejected.** b) agent files in pack-validation with agent-stack-spec and project needs in each repo's manifest; c) this monorepo.
**Tag.** [stated] · remind-me: mem_ad2c3b4877d64a0494d5bdfac712a55c
**Key.** lel:1571:profile-files-in-factory
**Source.** Nano, Let Em Loose project chat, message `cmsg_01WgyiM31RsSnZTgNuPX2VE4YGppB9nnPb7JnUCwC3s2Yq`, 2026-11-18T17:43:36Z: "1. Concur. 2. Concur. 3. Yes. 4. accept", answering in order the questions in thread message `cmsg_01WgyiM31RsSnZTgNuPX2VE42brap92o45tKyeDpvs4Be3` (~17:30Z). Item 4 answers #1563 and is recorded by its own thread, not here. Numbered D162 after D161.

## D163. #1166: reword the A2 fixture as "Proposed action, not run" (seed claim A stays the control); does not close Q53

**Decision.** Nano answered Q61: "Q61: yes. Reword A2 as 'Proposed action, not run: ...'. Also remove or rewrite 'after the dry-run list was reviewed': no dry-run list was reviewed, so the fixture itself states something that never happened. Leave seed claim A's wording unchanged, as the control. Record that this does not close Q53." The A2 text in `1166-seed-inbox.sh` becomes "Proposed action, not run: run `sweep --yes` on the toy data directory with DAYS 7." (built on branch `claude/lel-a2-wording`). It is a fixture choice within D146's scope: no threshold, roster or escalation rule changes. Seed claim A stays unchanged as the control, so the next run's evidence can compare the adversary's citations of msg 3 (A2) against msg 2 (A). Q61 is decided and closes once the PR is merged and deployed.
**Why.** The adversary cited A2 (msg 3) as a test run in smoke-3 (msgs 7, 12, 14), smoke-4 (msgs 12, 13) and smoke-6 (msgs 11 and 14, computing `7*86400` from A2's "DAYS 7"), and the fixture itself asserted a dry-run review that never happened.
**Rejected.** b) leaving the wording.
**Q53 stays OPEN.** This removes one trigger for invented citations. Invented runs, false "the brief mandates X" attributions and self or another-vote citations are separate.
**Tag.** [stated] · remind-me: mem_9e4daa70bddd471d9ffc49e3020a5fb6
**Key.** lel:1166:a2-fixture-reworded-proposed-not-run
**Source.** Nano, typed in the Claude Code session on BaileyAi, 2026-11-18, saved verbatim as `mem_0f6dd8deda1f4aceb7bfa9096941d65d` (speaker Nano). Relates to D146, Q53, Q61. Numbered D163 after D162 (D160 to D162 are the launch-profiles design in #1802).

## D164. Smoke-7 is the first run under D143; the D127 design freeze lifts; builder sessions (#1402, #600) are released first

**Decision.** Nano confirmed that smoke-7 is "the first run" of D127, as D143 defined it (a clean #1166 pass). The board confirms every D143 and section 9 criterion: claim 6 Accepted by the lenses at rev 72 (AcceptLine, score 10, two support authors), Nano's `choose` on question 1 dropped claim 7 at rev 118, the fixture A2 was dropped by Nano at rev 121, the inbox is empty, `bbp verify` is clean, and there were three candidates from three providers. The D127 design freeze LIFTS. The first thing it releases is the #1402 builder-session design rebuild (D129) and #600, which M2 depends on. The packs design (#723, D133, Q46 to Q48) and the Survey and Intent lenses (#1456), with the other `bbp_factory` Backlog cards, stay for later. The open items stay open: Q53, Q51, Q55, Q56 to Q58, Q60, Q62.
**Why.** Builder sessions are what the loop needs next, and D129 said they wait for the first run. Design approval, ask-first gates, spending and review/merge rules still apply to everything released.
**Rejected.** b) the packs design first; c) the Survey and Intent lenses and the other Backlog cards first.
**Tag.** [stated] · remind-me: mem_e2491f793d78419dbeeb0b764c33f4c4
**Key.** lel:127:freeze-lifted-smoke-7-first-run
**Source.** Nano, answer dialog in the Claude Code session on BaileyAi, 2026-11-18, saved verbatim as `mem_fff1f77dbd664befb0eb3945bd9c03b1` (speaker Nano). Lifts D127; applies D143; releases D129's wait. Numbered D164 after D163.

## D165. #600 builders: the Claude CLI in the v0 jail uses Nano's subscription (Q15)

**Decision.** The Claude CLI that a #600 builder session runs inside the v0 jail uses Nano's Claude subscription. Nano: "15 it uses my subscription." This answers which credential, not how it reaches the jail. The ask-first and D101 limits below still hold, and each needs its own answer before an account credential goes into `jail.json`:
1. **Delivery.** D101 says no secrets go inside the jail, only a short-lived token scoped to the lens and claim. A subscription login is an account credential. How it is reached (a gateway that holds it and honours an orch-minted token, or something else) is not decided. `jail.json`'s `token` {file, var} slot (D107) carries only an orch-minted token until it is.
2. **Spend.** D134 holds subscription calls back as paid. Builder sessions are paid model calls, ask-first, until Nano approves a run (D140 approved one lens run only). The Claude `no_overage` assertion is also not in `/etc/let-em-loose/assertions.json` (D140).
**Why.** Nano chose the credential source. The builder is Claude-only in v0 (`lel:600:claude-only-builders`, D64).
**Rejected.** None offered.
**Tag.** [stated] · remind-me: mem_f885f3a1b16a415eb01506c3146e0c7f
**Key.** lel:600:builder-claude-credential-subscription
**Source.** Nano, Millwright project thread for card #1402, message `cmsg_01632CBKCXsWe6RyFWvE9dovEBkpMdZ4enJ2R7MBLywZXz`, 2026-11-19T18:13:22Z: "15 it uses my subscription", answering OPEN_QUESTIONS Q15 (card #1318). Relates to D101, D107, D134, D140, `1571-launch-profiles-design.md` (model provider row). Numbered D165 because D163 and D164 were already taken (both now on main).
**Confirmed.** Confirmed by name 2026-11-19 by Nano ("confirm D165, D166, D167"), thread message `cmsg_01632CBKCXsWe6RyFWvE9dovXnfgsvDuk5meRkwzs4GR7M`, 2026-11-19T18:57:26Z, answering the PR #1824 review.

## D166. #600 v0: the Claude CLI reads Nano's `~/.claude` login through a read-only jail root (written exception to D101, `same_user` v0, expires with the gateway #726)

**Decision.** D101 stays the long-term rule: the gateway holds the login and the jail gets only an orch-minted token. For v0 only there is a written exception, scoped to `same_user` v0 (D161): the Claude CLI reads Nano's existing `~/.claude` login through a read-only root in `jail.json`. `jail.json`'s `token` slot stays orch-minted tokens only (D107). The exception expires when the gateway (#726) lands, and D101 then returns in force. Nothing is copied into `jail.json`. Nano accepted the caveat: v0 is not a security boundary and egress is open (the D107 gap), so a jailed builder could read and send the login. This answers the delivery half of D165 item 1.
**Why.** D165 chose the subscription as the credential; the gateway doesn't exist yet, and v0 runs as Nano's user (D161). A read-only root of an existing login needs no new credential file and no change to the `token` slot.
**Rejected.** A separate revocable subscription token file in the `token` slot: it would reverse D101 for good.
**Tag.** [stated] · remind-me: mem_c4e782acab594c2aa3cda4bf653f6aa2
**Key.** lel:600:subscription-login-v0-exception-same-user
**Source.** Nano, thread message `cmsg_01632CBKCXsWe6RyFWvE9dovNuNMU7fiwfx8bvmVJhWei4`, 2026-11-19T18:22:12Z: "concur", to the recommendation `cmsg_01632CBKCXsWe6RyFWvE9dov4g3WihbLKaDjCJDV35SQxg`. Follows D165; scoped by D161; limits D101 temporarily. Numbered D166 after D165 (D163 and D164 are on PR #1813).
**Confirmed.** Confirmed by name 2026-11-19 by Nano ("confirm D165, D166, D167"), thread message `cmsg_01632CBKCXsWe6RyFWvE9dovXnfgsvDuk5meRkwzs4GR7M`, 2026-11-19T18:57:26Z, answering the PR #1824 review.

## D167. One paid run approved: the #1318 toy-claim acceptance run (2 builders, 45 min, one claim) after the Claude `no_overage` assertion is added; #1320 not approved

**Decision.** Nano approves ONE paid run: the toy-claim acceptance run for #1318, with `builders_per_claim` 2, `builder_wall_secs` 2700 (45 minutes each) and one claim. This supersedes D134 for that run only, as D140 did for its run. Before it, the Claude `no_overage` assertion must be added to `/etc/let-em-loose/assertions.json` so the run fails closed. The dedup run (#1320) is NOT approved; it waits on M1 and the toy-run result, and any further paid call stays ask-first. The subscription is flat-rate, so the cost is usage limit, not dollars.
**Open action for Nano (done 2026-11-25).** Adding the `no_overage` assertion is a host write on BaileyAi; Nano did it himself. Nano: "Done", #732 project thread, message `cmsg_01WgyiM31RsSnZTgNuPX2VE4XwVns9yHdxFK1rGRYvyaUf`, 2026-11-25T19:02:38Z (usage credits off, account matches; remind-me `mem_5e7dc428234b4b71be3504e17bc8f228`). The entry is dated 2026-11-25 and lapses 2026-12-25: the toy run happens before then or Nano re-dates it. Entry shape (from `mem_fe5132a19f51456190c6017e94955e0a`): `{provider: claude, account: <email as `claude auth status` reports it for the running account>, checked: YYYY-MM-DD}`. The run did not start until it was in place.
**Why.** The smallest paid run that tests builder launch; the assertion makes it fail closed; the dedup run needs evidence from the first.
**Rejected.** Approving #1320 as well.
**Tag.** [stated] · remind-me: mem_77c0be9609544059ae911c7e2ccdb2f9
**Key.** lel:1318:one-paid-run-toy-claim-acceptance
**Source.** Nano, thread message `cmsg_01632CBKCXsWe6RyFWvE9dovNuNMU7fiwfx8bvmVJhWei4`, 2026-11-19T18:22:12Z: "concur", to `cmsg_01632CBKCXsWe6RyFWvE9dov4g3WihbLKaDjCJDV35SQxg`. Relates to D134, D140, D141, D165, `600-build-plan.md` section 4 defaults.
**Confirmed.** Confirmed by name 2026-11-19 by Nano ("confirm D165, D166, D167"), thread message `cmsg_01632CBKCXsWe6RyFWvE9dovXnfgsvDuk5meRkwzs4GR7M`, 2026-11-19T18:57:26Z, answering the PR #1824 review.


## D168. #703: the `bbp human --decision ID` and `bbp decision` CLI surface is approved

**Decision.** Nano approves the new bbp CLI surface in PR #1844 (card #703). `bbp human --decision ID` writes the human op with `by:<as>/dec:ID`; it rides the existing `op_accepted`, so there is no log or schema change. `bbp decision --task T --decision ID[,..]` is read-only and prints `[{decision,applied,rev}]`, where `rev` is the task revision just after the `op_accepted` that carries the id. Nano: "I approve". This settles the ask-first public-API gate (a new CLI surface) for this surface only. CI, review by `dottiusmaximus` and the merge rules still apply.
**Why.** Card #703 (decision ids for settling an uncertain send; QA.md, card #595 2026-11-03: backlog, not blocking v0) needs the board to learn whether a decision was applied, and a new CLI surface is a public API change.
**Rejected.** None offered.
**Tag.** [stated] · remind-me: mem_494a46793ab94f7aa5ea8eb891d99af4
**Key.** lel:703:bbp-human-decision-id-cli-surface
**Source.** Nano, Mill Board project thread, message `cmsg_017MoLbtrGAAcyE1PVPZXuN63jqD6m4iZ3CnxC8vRdurRk`, 2026-11-24T08:11:19Z: "I approve", replying to the caller's request to approve that CLI surface (relayed by the caller; the request message itself was not read). Applies D119's host-CLI trust model (`bbp human`); fulfils the QA.md § card #595 deferral of decision ids to #703.

## D169. #703: the board-side API changes in PR #1844 are approved as built

**Decision.** Nano approves, as built, the board-side API changes in PR #1844 beyond the CLI surface of D168: `Escalations::act` takes an optional decision id, `Escalations::applied` is added, and the `inbox_act` answer carries `settled`. This settles the ask-first public-API gate for these changes only. CI, review by `dottiusmaximus` and the merge rules still apply.
**Why.** Card #703 needs the board to learn whether a decision was applied; these are the board calls that carry it, and a public API change is ask-first.
**Rejected.** None recorded (the card's other options were not relayed).
**Tag.** [stated] · remind-me: mem_bcf96446f7ab4d679867c861a8cc8dca
**Key.** lel:703:board-api-changes-as-built
**Source.** Nano tapped "Approve as built" on a Hearth decision card, 2026-11-24 (card `cmsg_017MoLbtrGAAcyE1PVPZXuN6XmhpUb1QuGg8qTvDpSxKzt` or `cmsg_017MoLbtrGAAcyE1PVPZXuN6DEW14f4wpgLziio54sPHnY`; taps arrived as messages at 14:37:23Z and 14:37:08Z; the caller did not say which card carried which tap). Relayed by the caller; the cards were not read. Extends D168.

## D170. #703: an uncertain inbox send is written ahead as a pending mark; closed as not applied only on proof

**SUPERSEDED by D171** (2026-11-24): the write-ahead pending marks were removed from PR #1844.

**Decision.** For an inbox send the board cannot prove was tracked, the board persists a pending mark on the Remediate card before calling BBP. It clears the mark only on proof: the decision id appears in the log, or BBP refused before writing. It never closes the card as not applied while a mark is open. Implemented in commit `ede5d038b` on branch `claude/project-thread-9w7nhi` (PR #1844).
**Why.** Closing as not applied on an unproven send could hide a decision that was in fact applied; the write-ahead mark keeps closes rare and safe.
**Rejected.** The other options on the card were not relayed; Nano chose "Write-ahead pending, rare closes".
**Tag.** [stated] · remind-me: mem_924abcd166b64ab8a1639b59530abe4a
**Key.** lel:703:uncertain-send-write-ahead-pending
**Source.** Nano tapped "Write-ahead pending, rare closes" on a Hearth decision card, 2026-11-24 (cards `cmsg_017MoLbtrGAAcyE1PVPZXuN6XmhpUb1QuGg8qTvDpSxKzt` and `cmsg_017MoLbtrGAAcyE1PVPZXuN6DEW14f4wpgLziio54sPHnY`; taps at 14:37:23Z and 14:37:08Z). Relayed by the caller; the cards were not read. Builds on D168.

## D171. #703: Mill Board never closes a Remediate card from BBP's log; it only notes an applied decision

**Decision.** For PR #1844 (card #703) Nano chose "Drop auto-close, keep applied note". Mill Board never closes a Remediate card from BBP's log. When the log shows the decision id applied it adds one note on the card with the rev; otherwise it leaves the card for a person. The write-ahead pending marks of D170 are removed from the PR. The CLI (`bbp human --decision ID` with `--fence`, `bbp decision`) and the `Escalations::act` / `Escalations::applied` API stay as approved in D168 and D169. Supersedes D170.
**Why.** The reviewer found a race in each close path, and the typed-state alternative was a card schema change (ask-first). Never closing from the log is the simplest safe behaviour.
**Rejected.** Write-ahead pending marks with proof-only closes (D170); a typed card state (card schema change). The remaining card options were not relayed.
**Tag.** [stated] · remind-me: mem_844eeb2e2a6343eeb09f431eea1099dc
**Key.** lel:703:drop-auto-close-keep-applied-note
**Source.** Nano tapped "Drop auto-close, keep applied note" on Hearth decision card `cmsg_017MoLbtrGAAcyE1PVPZXuN6Y4PcGdaZac4qsyWyRT4Jo5`, 2026-11-24T16:03:01Z. Relayed by the caller; the card was not read. Supersedes D170; leaves D168 and D169 as approved. Q63 (is required `--fence` acceptable) stays open.

## D172. #703: the PR #1844 surface is approved as built, including the required `--fence`

**Decision.** Nano approves the surface listed in the PR #1844 review (6043258136), in particular the required `--fence` (Q63, answer a): `bbp human --decision ID` now requires `--fence`; `bbp decision` counts only the human principal; `Escalations::act(decision)` and the new required trait method `Escalations::applied`; `inbox_act`'s `recorded.settled` (`applied {rev}` or `unknown`); a stale-rev retry that finds a card returns `recorded`; the second card id scheme `tracked_id`; `decision::lookup(human)` and `human::op_for`. Nano did not say anything about dropping the second id scheme, so it stays as built. This settles the ask-first public-API gate for this surface only. CI, review by `dottiusmaximus` and the merge rules still apply. Extends D168 and D169 (the required `--fence` is stricter than D168's shape); consistent with D171 (no auto-close).
**Why.** A decision id without a fence could skip the revision fence, so the stricter shape is the safe one; the other items are public API of the board and BBP, which is ask-first.
**Rejected.** Making `--fence` optional to match D168's shape (Q63 option b).
**Tag.** [stated] · remind-me: mem_f6abd383d5b9495c9e796c271eb42140
**Key.** lel:703:approve-pr1844-surface-required-fence
**Source.** Nano, GitHub comment by `baileyrd` on Rusty-Mill/rusty_mill#1844, comment id 7137833262, 2026-11-24T16:32:51Z: "I approve the surface it lists, in particular the required --fence (Q63)." Replying to review 6043258136. Relayed by the caller; the comment was not read.

## D173. #1416: the LEL tab in rusty_term reads BBP through the Mill Board HTTP API (inbox via `inbox_list`/`inbox_act`; new read endpoints for claims, intent and spend)

**Decision.** The LEL tab on Nano's desktop reaches BBP on the orch host through Mill Board's HTTP API. The inbox reads through `inbox_list` and acts through `inbox_act`, with the same revision fence and `dry_run` (D35). Claims map, intent and spend need NEW read endpoints on Mill Board that wrap `bbp`. Those endpoints are a public API change: they get their own PR and an owner gate before slices 2, 5 and 6 of `1416-lel-tab-slice-plan.md` can show live data. Slice 1 (the view model) does not depend on them. No BBP or orch change (D55). Numbered D173 because D168 to D172 are taken by #703.
**Why.** Smallest option: one writer path and one fence; `bbp` and the log live on the orch host while rusty_term runs on the desktop.
**Rejected.** (b) `bbp` over SSH or a mux block (waits on Windows `--ssh`, C79, and credentials); (c) a new mux `Features` call proxying read-only BBP views and the actions (larger protocol change).
**Tag.** [stated] · remind-me: mem_4366993b5c9f4930b82a48b3f0542bad
**Key.** lel:1416:lel-tab-data-source-mill-board-http
**Source.** Nano, decision card in the project thread, message `cmsg_014f6o52E2iAAAgMm9v8hh1wSELECQZe1co7b86n4Fniua`, 2026-11-24T08:07:40Z: tapped "Mill Board HTTP API", answering "How should the LEL tab on your desktop reach BBP on the orch host?". Relates to D35, D53 to D58, `1416-lel-tab-slice-plan.md`.

## D174. #1416: the LEL tab view model and parsing live in rusty_term, fixtures from `bbp inbox --json` plus a drift test; no shared crate

**Decision.** rusty_term does not depend on `rusty_mill_board` (sqlite, oauth, serve; ADR-0003 keeps `bbp` a process boundary). The view model and parsing live in rusty_term, with golden JSON fixtures copied from `bbp inbox --json` and a drift test that parses the same fixtures in both crates. Extracting a shared crate is a new crate and needs owner approval; it is not done.
**Why.** Most reversible: no new crate and no heavy dependency in the terminal; a shared crate can be extracted later without changing the wire format.
**Rejected.** A shared types crate now (owner gate); depending on `rusty_mill_board`.
**Tag.** [default] · remind-me: mem_f0631ddd540840a0b878c4bc2590bb88
**Key.** lel:1416:lel-tab-types-in-rusty-term
**Source.** let-em-loose-expert default, 2026-11-24, for the Types section of `1416-lel-tab-slice-plan.md`. Follows D173.

## D175. #1092: the inbox runs on a second Mill Board on BaileyAi with its own SQLite file; rusty_tick retired, live board untouched

**Decision.** rusty_tick is retired. The live Mill Board (its own SQLite file) stays in use and is not touched by #1092. The #1092 inbox is served by a second, separate Mill Board on BaileyAi with its own SQLite file, `MILL_BOARD_DB=/var/lib/mill-board/board.db`, and no tick: `MILL_BOARD_TICK_URL`, `RUSTY_TICK_TOKEN` and the loopback tick on `127.0.0.1:8788` are dropped from `board.env` and `1092-deploy-layout.md`. Cards, including #1092, stay on the live board, so a card's link and the inbox it points at are on different boards. D132's `mill-board` user, `/var/lib/mill-board` state, root-owned wrapper, guard and sudoers rule stand unchanged. #1092's done-when (`inbox_list` shows a linked task's escalations, `inbox_act` writes the log) is checked on the BaileyAi board.
**Why.** With the tick gone and the live board staying where it is, a board on the orch host keeps D35's rule (Mill Board reads BBP only through the host's `bbp`, so it runs on the orch host) and D132's guard as the only door to `bbp`, with no board code change and no remote path into `orch-runner`.
**Rejected.** a) The live board reaching BaileyAi's `bbp` over an SSH forced-command key into the D132 guard (one board, but a new remote trust boundary and a guard change). c) Moving the live board to BaileyAi.
**Tag.** [stated] · remind-me: mem_72a5c92bf1a147acaae78dae66d4c2d1
**Key.** lel:1092:inbox-second-board-baileyai
**Source.** Nano, Let Em Loose thread, 2026-11-25: "rusty_tick is retired" (`cmsg_01WgyiM31RsSnZTgNuPX2VE4BFfyyjHrARFenqpKUHKWkn`, 01:22Z); "Millboard is still in use" (`cmsg_01WgyiM31RsSnZTgNuPX2VE4ELuF7quQwJY6RZ9pftamw9`, 01:26Z); tapped "Second board on BaileyAi" on decision card `cmsg_01WgyiM31RsSnZTgNuPX2VE4XgzhpjMgWg7FpLVAyUKAmc`, 01:26:50Z, answering the expert's escalation. Relayed by the coordinator; the messages were not read. Confirmed by Nano in a GitHub comment by `baileyrd` on Rusty-Mill/rusty_mill#1857 (comment id 7164514947, 2026-11-25T13:56:36Z): "D175 confirmed: the #1092 inbox runs on a second Mill Board on BaileyAi with its own SQLite file, rusty_tick is retired, and the live board is untouched." Supersedes D132's tick dependency only (its `board.env` and `1092-deploy-layout.md` tick lines); amends D35's single-board picture for v0. Within D130 and D131. Numbered D175 because D173 and D174 are taken by #1416.

## D176. #430: rusty_bbp's co-edit fold moves onto rusty_crdt as a pure swap

**Decision.** Nano gave the go for #430 as option 1, "pure swap, keep the refusal". `rusty_bbp::coedit`'s fold uses `rusty_crdt`'s `Rga` as its core. Admission (the overlap/stale refusal) is unchanged, golden logs stay byte-identical, and a differential test against the old fold proves the swap, in one small PR. This supersedes the card's older wording that the swap "would change admission only". This settles what to build; CI, review by `dottiusmaximus` and the merge rules still apply.
**Why.** It gives `rusty_crdt` its first user and puts the fold on a law-tested core without changing BBP log semantics.
**Rejected.** Option 2, swapping and accepting overlapping edits (CRDT admission; changes BBP log semantics; ask-first). Option 3, dropping the card.
**Tag.** [stated] · remind-me: mem_8d894a129917493782f8b8f1e8123b26
**Key.** lel:430:coedit-fold-on-rusty-crdt
**Source.** Nano, project thread "Update Mill Board card statuses", message cmsg_014AqbZs58L565hMV2pFkCX5HGJTsB1QUY7avVobhzUMnP, 2026-11-25T01:15:31Z: "1", answering the three options in cmsg_014AqbZs58L565hMV2pFkCX5C5P4veGi9BERQNHB1fH2T3. Relayed by the caller; the message was not read.

## D177. #1404 build defaults from the recovered #600 design

**Decision.** The approved #600 design (`600-builder-session-design.md`, D66) survived in `/mnt/project-files/let-em-loose/` and is now in this folder (PR #1874), so OPEN_QUESTIONS Q44 was wrong that it was gone. It settles what `600-build-plan.md` marked **[filled]** for #1404:
1. **Config defaults** are the approved ones (design §5): `builders_per_claim` = 1, `max_live_builders` = 3, `builder_wall_secs` = 2700. The plan's [filled] 2 and 2 are withdrawn. D167's two builders is run config for the #1318 toy run only, not a default.
2. **Admission** (design §4.1): an open design claim with no live candidate, or whose latest candidate failed, and `spent(c) < budget(c)` read from the task. A spent budget gets the existing budget refusal; no new budget rule.
3. **`builder_session` record**, a new orch-store record type with its own schema tag (D66 cleared the ask-first gate for this table, and no existing data is migrated): `claim`, `n`, `candidate` (set when stored), `state` (`admitted`, `running`, `stored`, `failed`, `aborted`), `started`, `ended`, `tokens_in`, `tokens_out`, plus optional per-profile run results and erosion delta, filled when the run report lands (design §6). Wall time is derived as `ended - started`, not stored, so it has one source. `human_touched` (D106) and the claim's later outcome (accepted, reversed, reopened) are derived from the claim fold, not stored. (Amended 2026-11-25: `wall_secs` dropped as a stored column to match the built row, coordinator report from the #600 thread.)
**Why.** Everything here sits inside D66's approved design. The column layout of an approved orch-internal record is implementation detail, and the derived flags follow D106.
**Rejected.** The build plan's [filled] 2/2 defaults (they contradict approved §5). Taking D167's run values as defaults. A stored `human_touched` column.
**Tag.** [default] · remind-me: mem_0a48ad5089f14c64aebb63790a976736
**Key.** lel:1404:builder-session-build-defaults
**Source.** Caller question from the #600 thread, 2026-11-25; `600-builder-session-design.md` §4.1, §5, §6, §8 (approved, D66); D106; D167.

## D178. #1405: how Windows-only intent lines are recognised

**Decision.** The open-time `untested` check is an orch check (approved design §2, §8 item 1), and BBP needs no change (§8 item 4), so there is no new event field. A line is Windows-specific when it is listed in a mapping file in the task's frozen inputs, keyed by intent line; each entry names a `golden:<case>`, `shadow`, or nothing. At open, orch lists every listed line with neither a golden case nor a shadow run as `untested` for the Linux runs. There is no marker in `INTENT.md` (frozen; any change is a new version) and no text heuristic. Which lines go in the file is experiment data that goes to Nano with the golden-set sign-off (Q4); the check is tested with fixtures.
**Why.** The approved design already requires each Windows line to map to a golden case or the shadow run; a frozen-input mapping file is the most reversible way to recognise those lines without touching the frozen intent or the BBP schema.
**Rejected.** A marker in `INTENT.md`. Matching on the line's text. A BBP host check with a new event field (ask-first, and §8 says BBP: none).
**Tag.** [default] · remind-me: mem_8515601b9f5d426e973dcad3c0524df4
**Key.** lel:1405:windows-line-recognition
**Source.** Caller question from the #600 thread, 2026-11-25; `600-builder-session-design.md` §2, §8 (approved, D66); D56, D63.

## D187. #1318 v0 builder launch defaults

**Decision.** Implementation choices for `1318-builder-launch-slices.md` (draft, awaiting Nano's approval), inside the approved designs (D66, D104, D107, D108):
1. **Permission.** orch answers every ACP `session/request_permission` with the offered `allow_once` option (`allow_always` only when no `allow_once` is offered) and writes the request and the answer to `acp.log`.
2. **Usage.** `tokens_in`/`tokens_out` come from the adapter's reported usage; when it reports none they are 0 and `acp.log` says "usage not reported". Before #596 compares token numbers, an unreported run must be told apart from 0 (a later change to the `builder_session` row, D177).
3. **Viewer.** The display-only session block runs `tail -F <dir>/builders/<claim>/<n>/acp.log`, outside the jail, held after exit. No new viewer binary.
4. **Mirror.** Its contents are fixed by the task's frozen inputs (600 design §4.3). The toy claim needs no external dependencies, so its mirror is an empty read-only root. For #1320: a read-only vendored directory made from the experiment repo's lock file at task open.
5. **Token.** v0 `jail.json` has no `token` (optional, D107) until orch mints one in #1382; nothing inside the jail consumes one before the gateway.
**Why.** A permission prompt is not a boundary; the jail confines and built-in tools stay on (D101, spike #727); `allow_once` keeps every request in the log. Zero usage is reversible. D104 item 4 says a viewer program is enough. The #726 design builds the minter last (726-sandbox-design.md §10, D180).
**Rejected.** Denying or escalating permission requests; `allow_always` (the adapter stops asking and the log loses entries); an optional token column now (changes the #1404 row in the queue); a new viewer binary; a minted token with no consumer.
**Tag.** [default] · remind-me: mem_98aa01df7ad849d2b9643cb5f5137207
**Key.** lel:1318:builder-launch-v0-defaults
**Source.** Caller question from the #600 thread on `1318-builder-launch-slices.md` §5, 2026-11-25; D66, D101, D104, D107, D177, D180; `600-builder-session-design.md` §4.3; `mux-hosted-agents-design.md` §6. Numbered D187 because D179 to D186 are taken on open PRs (#1871, #1872, #1873, #1877).

## D188. #1318: slice design approved; a candidate reaches BBP on `end_turn` + `.lel/submit` + non-empty diff

**Decision.** Nano approves `1318-builder-launch-slices.md` (S1 to S6, with D187's defaults) and picks option a for how a builder's candidate reaches BBP. After the turn, orch posts the diff and candidate under the builder's seat only when all three hold: the stop reason is `end_turn`, the builder wrote the submit marker `.lel/submit` in its worktree, and the diff is non-empty. Otherwise orch posts nothing, nothing is debited, and the session ends `failed`. This closes the §5 point that blocked S5.
**Why.** The marker tells a deliberate submission apart from a builder that gave up, so half-done work never becomes a debited candidate (600 design §4 step 7). orch already decides what reaches the board (D104), and option a adds no tool surface before the #726 gateway.
**Rejected.** b: post on any `end_turn` with a non-empty diff, even when the builder gave up. c: a BBP submit tool offered to the agent over ACP (an MCP server in `session/new`), which means more surface to build and test in v0. "Not yet, I have changes".
**Tag.** [stated] · remind-me: mem_22f4ae32277a47aeb0ba09fa4deed541
**Key.** lel:1318:design-approved-candidate-submit-marker
**Source.** Nano, decision card in the #600 project thread, message `cmsg_01WgyiM31RsSnZTgNuPX2VE48sBVZYhJPE5kniVhcHSwk1`, tapped 2026-11-25T17:26:33Z: "Approve, marker + diff (a)". The question was "Approve the #1318 design, and how should a builder's candidate reach BBP?" This satisfies D25 for #1318. Review, merge and ask-first gates (the new public surface in §5) still apply per PR. Relates to D66, D104, D177, D187. Numbered D188 because D179 to D187 are taken on open PRs (#1871, #1872, #1873, #1877, #1880).

## D189. #1318 S5 `rusty_orch builder run` implementation defaults

**Decision.** Inside the approved S5 (D188, `1318-builder-launch-slices.md` §S5):
1. **Flags.** Beyond `<config> --bbp --dir --task --claim --state`: `--repo`, `--base`, `--cgroup-parent` (a delegated cgroup dir holding the `lel-store-<scope>` name), `--slots`, `--mux-ctl`, optional `--mux-socket`. No orch config schema change. `--base` is the caller-passed task baseline (the protected set's `baseline`) until 600 §4 step 2's claim-base rule is coded, then it becomes optional. The flags ride the command's existing ask-first note in the PR.
2. **Holder deadline.** `--deadline-ms` = now + `builder_wall_secs` + 60 s grace + 30 s margin, so orch's end path (term, grace, `cgroup.kill`, `release`) always runs first and the holder's kill stays the backstop for a dead orch (§S5 step 6).
3. **fd 4 to stdio.** Jail program `/bin/sh` with `-c 'exec claude-code-acp <&4 >&4 4>&-'`; stderr stays on the pty; `/bin` and `/usr` in read roots. Closing fd 4 keeps the agent's tool subprocesses off the ACP channel. `lel-jail` is unchanged.
4. **Recover.** `builder run` does not call `orch_lens::builder::recover`; #1320's loop owns it, since a sibling `builder run` (D167, `builders_per_claim` 2) would be aborted. A crashed run's row stays `running` and counts toward `max_live_builders` until then.
5. **Prompt.** A short built-in prompt: claim id and text, the worktree, "build it; run the visible tests as often as you like; when it is ready write `.lel/submit` and end your turn; to give up, end your turn without it". `.lel/` is excluded from the posted diff. Failing-profile names (600 §4 Retries) are added when the card carries them.
6. **Seat and posts.** Through `bbp mcp` with `BBP_PRINCIPAL=claude-builder` (the agent file's id; it must be the task's assigned builder seat, addendum C-R1) and `BBP_SESSION`, as lenses do. Diff then candidate, op ids `builder-<claim>-<n>-diff` and `builder-<claim>-<n>-candidate` (`n` is unique per claim from `admit`), so a lost answer replays. Diff: `git add -A` excluding `.lel`, then `git diff --cached --binary <base>`.
7. **Store.** Each step opens the store briefly (load, replace this row, save), retrying briefly on `DirLock` `Held`; admission is one such step. Rows saved at admitted, running and ended.
**Why.** Each is implementation detail inside the approved design and the most reversible choice: no config schema, BBP or merged-crate change, and the holder deadline keeps the design's stated order.
**Rejected.** Deadline at exactly the wall (the holder kill would always beat orch's end path). Teaching `lel-jail` to dup fd 4 onto 0/1 (a change to merged `orch-jail` for what a wrapper does). Calling `recover` at startup (aborts a live sibling). New orch config fields for the launcher paths.
**Tag.** [default] · remind-me: mem_f09b470e92384f13befd1e1adce9a0ed
**Key.** lel:1318:s5-builder-run-defaults
**Source.** Caller question from the #1318 S5 implementation, 2026-11-25; `1318-builder-launch-slices.md` §S5; D104, D167, D177, D187, D188; addendum C-R1. Numbered D189 because D179 to D188 are taken on open PRs (#1871, #1872, #1873, #1877, #1880).

## D196. #724: the redrawn `factory-map.svg` is accepted as the Factory map for now

**Decision.** `docs/projects/let-em-loose/factory-map.svg`, redrawn from card #724's description of what changed (Q45's interim choice) in PR #1876, is accepted as #724's Factory map for now. The original Factory map stays untouched. "For now": a later redraw, or the 2026-11-04 voice-chat source SVG if it is recovered, may replace it without a new decision on intent. This settles the drawing only; CI, review by `dottiusmaximus` and the merge rules still apply to PR #1876.
**Why.** The source SVG from the 2026-11-04 voice chat was not reachable from a repo session; Nano reviewed the redraw on the PR and found it good enough.
**Rejected.** Waiting for the source SVG before landing #724's map.
**Tag.** [stated] · remind-me: mem_b5e76a2c6dde43489fec5e501929dd57
**Key.** lel:724:factory-map-redraw-accepted
**Source.** Nano, project thread message `cmsg_01WgyiM31RsSnZTgNuPX2VE4YM1e34fpJeXfgbUJqq3Mf5`, 2026-11-25T20:13:53Z: "Goood enough for now", after reviewing the drawing on PR #1876. Relayed by the caller; the message was not read. Resolves OPEN_QUESTIONS Q45. Numbered D196 because D177 to D195 are taken on `main` and open branches.

## D198. #1416 slice 2: claim field bindings, stage strip derivation rule, brief text as a plain input

**Decision.** (1) The LEL tab binds to the merged `rusty_bbp::claims::ClaimView` names (`id, text, domains, status, score, accept_at, confidence, epoch, spent, budget, deps, candidates, sole_candidate, human_touched, holdout_due, failing, intent{current,proven_at}`). Ids are plain integers on the wire, `status` is PascalCase, and `intent` is absent until the intent is first revised. Parsing ignores unknown fields (`evidence`, `tests_changed`, `endorsement`) and defaults `accept_at`, `human_touched`, `holdout_due` and `intent` when absent. `evidence` is parsed in the slice that shows it (D58 top-two items). (2) Stage strip, derived per snapshot with no stored state. A claim is live when its status is Open, Accepted or Escalated. Intent is done as soon as the task has any claim (the brief text is not needed: it has no source until D173). Skeletons is done when a live claim has a candidate (`current` set: the builder's stored candidate; `candidates` lists a question's answer claims, not skeletons, and only feeds the stuck-question display). Under attack is done when at least one claim is Accepted and none is Open or Escalated. The strip never contradicts itself: a later stage done marks the earlier ones done, and exactly one of the first four stages is current (the first not done). Shadow run is the current stage once Under attack is done and is never shown done: the human's real-drive reality check leaves no BBP fact. Living app is always pending, labelled "not built" (same as the Q41 undo log). With no claims, Intent is current (a task with only dropped or void claims has Skeletons current) and the tab shows "new, gathering". Each stage shows text plus glyph, no animation (gui D55). (3) Slice 2 takes the brief text as a plain input (`Option<String>`). The Mill Board read endpoint that serves brief text is part of the gated API PR (D173, Q64) and its shape waits.
**Why.** The names come from the merged struct; the stage rule uses only claim facts D116 allows and cannot claim a shadow run happened. A plain-text input keeps slice 2 testable on fixtures.
**Rejected.** A stage field in BBP or orch (D116). Marking Shadow run done from a claim domain name nobody defined. Reading `candidates` as skeletons and Intent pending without a brief (both corrected on review of PR #1896). Fetching the brief artifact from rusty_term (no read path exists on the desktop).
**Tag.** [default] · remind-me: mem_0d5e5a974ebd43479d6d75fcf6faeed8
**Key.** lel:1416:lel-tab-slice2-bindings-stage-rule-brief-input
**Source.** Caller question on #1416 slice 2 (Q9 of `1416-lel-tab-slice-plan.md`), 2026-11-25; `rusty_bbp` `src/claims.rs` `ClaimView`, `IntentView`, `EvidenceView`; `rusty_mill_board` `src/inbox.rs` (`brief` is an artifact id, claim JSON sample); D116, D173, D174; README Phases 2 to 5. Note: "D57" in the plan is gui-design D57, not this log's D57.

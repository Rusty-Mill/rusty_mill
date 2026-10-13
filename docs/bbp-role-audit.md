# BBP role-name audit (spec v0.4.1, `rusty_bbp`, `rusty_bbp_host`)

Step 1 of the v0.5.0 role-extensibility work. Audit only: nothing in the spec or engine is edited yet.

**Target.** BBP defines a generic `Role` (id + capability set) and enforces the roster it receives. It never names a role. Concrete roles are built where the agent is built (orch) and handed over as a runtime roster.

**Classes.** (a) generic capability the `Role` type needs; (b) role-specific rule that belongs in orch/host; (c) example or illustrative text, moved to a non-normative appendix.

**Scope.** Current spec `docs/spec/bbp-v0.4-implementation-spec.md`; `crates/libs/protocol/rusty_bbp` (src, tests, TRACES.md); the consumer `crates/apps/rusty_bbp_host`. Spec drafts v0 to v0.3 and `docs/review/*` are history and stay untouched. v0.5.0 is a new file, matching how v0.1 to v0.4 were kept.

## Capability set the audit converges on

The hardcoded role rules collapse into nine fields. Every proposed replacement below uses these names.

| Field | Meaning | Replaces |
| --- | --- | --- |
| `id` | `RoleId`, validated non-empty token | `enum Role` |
| `turn_order` | `u16`; lowest wins when several roles qualify for a state's default turn | `State::default_role` |
| `post` | set of `MessageKind` the role may post | `role_may_post`, capability matrix |
| `write` | set of agent-writable `ArtifactKind` (`spec`, `diff`, `candidate`); the state each kind is writable in stays a protocol rule | `gate_artifact` writer column |
| `read_artifacts` | set of `ArtifactKind` | `readable()` |
| `read_messages` | `All` or `Scoped` (human messages addressed to the role or `*`, plus decisions and answers about its own messages) | `visible()` |
| `verdict_at` | subset of `{test, review}` stages the role may judge | `check_verdict`, T9 to T11 |
| `final_decision` | its `approve` at `review` is required to reach `merge_gate` and for the human to approve the merge | "a Reviewer approve" |
| `consultable` | may receive a yielded turn | consultation table |

Default turn holder per state, derived: `planning` = lowest `turn_order` with `write` containing `spec`; `build` = `candidate`; `test` = `verdict_at` containing `test`; `review` = `verdict_at` containing `review`. The plan gate is opened by a role that may write `spec`. A `Scoped` reader may answer only the human (derived, no extra flag).

`human`, `moderator` and `runner` stay protocol principal kinds, not roster roles: they carry fixed authority the store checks (gates, accounting, run secret).

## A. Spec (`bbp-v0.4-implementation-spec.md`)

| # | Location | Current text | Class | Proposed replacement |
| --- | --- | --- | --- | --- |
| S1 | L21 | "the Reviewer can answer a human" (v0.3 to v0.4 change list) | c | Drop. v0.5.0 gets its own change list; v0.4 history stays in the v0.4 file. |
| S2 | L61 | principal record carries "a `role` and a `vendor`" | a | `role` is a `RoleId` present in the task roster. |
| S3 | L68 | `Planner, Coder, Tester, Reviewer \| messages and artifacts per the matrices` | a | `Agent (any rostered role) \| messages and artifacts its capabilities allow`. |
| S4 | L74 | "The Reviewer's session is new for each candidate it judges." | b | Host requirement becomes roster metadata owned by orch ("fresh session per candidate"); BBP cannot verify it. Spec keeps the generic "one agent session per role per task". |
| S5 | L109-111 | gating table Writer column: Planner (`spec`), Coder (`diff`, `candidate`) | a | "role with `write` containing the kind"; the State and Credential columns are unchanged. |
| S6 | L154-155 | message example `"from": "tester", "to": ["coder"]` | c | Neutral ids (`"from": "r2", "to": ["r1"]`); the concrete names appear in the appendix roster. |
| S7 | L174, L181-187 | `verdict` ... "Tester, Reviewer"; by-state table cells "Tester" (`test`), "Reviewer" (`review`) | a | "roles whose `verdict_at` contains the state's stage". |
| S8 | L247 | card example `"turn": {"role": "tester", ...}` | c | Neutral id. |
| S9 | L281 | R6: "`verdict` comes from Tester (in `test`) or Reviewer (in `review`)" | a | "`verdict` comes from a role whose `verdict_at` contains the current stage". |
| S10 | L292 | R17: "a Reviewer `approve` naming the same candidate and run exists" | a | "an `approve` from a `final_decision` role at `review`, naming the same candidate and run, exists". |
| S11 | L301-309 | Capability matrix: four named rows plus Runner | a / c | The matrix becomes the generic `Role` schema (field table above) plus the roster-injection section. The four named rows move to the appendix as an example orch roster. The Runner row stays (principal kind). |
| S12 | L319 | "`planning` Planner, `build` Coder, `test` Tester ..., `review` Reviewer" | a | Default-holder derivation by capability and `turn_order`. |
| S13 | L320-328 | consultation table (Coder/Tester, Planner/Tester, Planner/Coder, "Reviewer, by human ask only") | a | Rule: a role is yield-consultable in a working state when `consultable` is set and it is not that state's default holder. A human `ask` may address any assigned role in any non-terminal state (the engine already does this; the table understated it). |
| S14 | L349-352, L368-369 | T9 "Tester `approve`", T10 "Tester or Reviewer `revise`", T11 "Reviewer `approve`", resume guards "Tester `approve` exists", "Reviewer `approve`" | a | T9: `approve` at stage `test`; T10: `revise` at stage `test` or `review`; T11: `approve` at stage `review`; guards likewise, merge guard needs a `final_decision` approve. |
| S15 | L377-381 | entry actions "grant Planner / Coder / Tester / Reviewer" | a | "grant the state's default holder". |
| S16 | L446 | Risk "Reviewer isolation hides settled context" | a | "`Scoped` readers may miss settled context. Measure first." |
| S17 | L455 | Q3 "whether the Reviewer's per-candidate fresh session can relax" | b | Moves to the orch follow-up note (it is a roster/host policy question). |
| S18 | L15 | "Claude, ChatGPT/Codex, Gemini, Ollama and Hermes agents" | c | Vendor names, not roles; Context paragraph becomes generic, names move to the appendix. Flagged for completeness. |

## B. Engine (`crates/libs/protocol/rusty_bbp/src`)

| # | Location | Current text | Class | Proposed replacement |
| --- | --- | --- | --- | --- |
| E1 | record.rs:7-16 | `enum Role { Planner, Coder, Tester, Reviewer }`, `Role::ALL` | a | `RoleId(String)` newtype (validated), `Role { id, turn_order, caps }` struct, `Roster` (validated, below). Delete `ALL`. |
| E2 | record.rs:23, 160, 218, 258; event.rs:41,47,51,79; command.rs:13; state.rs:128,131,151,158,171,201 | `Role` as identity in `PrincipalKind::Agent`, `Recipient::Role`, `yield_to`, `from_role`, events, `Turn`, `assigned: HashMap<Role,_>` | a | `RoleId`. |
| E3 | state.rs:41-48 | `default_role`: Planning to Planner ... Review to Reviewer | a | `Roster::default_holder(state)`. |
| E4 | state.rs:51-58 | `consultable`: per-state `matches!` on Coder/Tester/Planner | a | `Roster::consultable(state, role)`. |
| E5 | state.rs:322 | `verdict(role, kind)` lookup | a | `verdict_at(stage, kind)`; callers pass a stage, not a role. |
| E6 | engine.rs:293-311 | `visible()`: `role != Reviewer` returns early; `to_reviewer`, `from_role == Reviewer` | a | `read_messages == Scoped` with the role's own id. |
| E7 | engine.rs:313-315 | `readable()`: Reviewer cannot read `Log` | a | `read_artifacts` contains kind. |
| E8 | engine.rs:395-415 | `gate_artifact`: `(Spec, Planner, Planning) \| (Diff/Candidate, Coder, Build)` | a | `write` contains kind, with the per-kind state binding kept as a protocol rule. |
| E9 | engine.rs:535-548 | `role_may_post`: four-arm match | a | `post` contains kind. |
| E10 | engine.rs:646-653 | `check_verdict`: `(Tester, Test) \| (Reviewer, Review)` | a | `verdict_at` contains the stage of the current state. |
| E11 | engine.rs:742-747 | Reviewer may answer only the human; message "reviewer answers only the human" | a | `Scoped` reader may answer only the human; message names the capability. |
| E12 | engine.rs:823-828 | "only the Planner opens the plan gate" | a | `write` contains `spec`, state `planning`. |
| E13 | engine.rs:848-860, 1297-1304 | `check_yield` and request-settlement regrant via `consultable` / `default_role` | a | Roster methods (E3, E4). |
| E14 | engine.rs:1026-1029, 1215-1222 | `verdict(Reviewer, Approve)`, `verdict(Tester, Approve)`; messages "no Reviewer approve", "review needs a Tester approve", "merge_gate needs a Reviewer approve" | a | `verdict_at(Test)` / `final_decision` approve; messages name the capability, not a role. |
| E15 | engine.rs:1339 | entry-action grant uses `default_role` | a | `default_holder`. |
| E16 | engine.rs:490 | comment "grant a Coder turn" | c | "grant the build turn". |
| E17 | record.rs:101 | doc comment "Reviewers read it" | c | "Roles with `read_artifacts` containing `test_report` read it". |

Already role-neutral and unchanged: R9 recipient check (`st.assigned.contains_key`), human-ask grant (engine.rs:1311), `grant()` itself.

**Log format.** `Role` is serialized into events (`Assigned`, `TurnGranted`, ...). Variant name to string changes the on-disk shape, and to replay from the log the roster must be in the log too. Proposal: the roster is recorded in `task_opened`, immutable for the task. Existing logs (proving run) will not replay on v0.5.0; there are no logs to keep, but this is the one irreversible change, so flagging it now.

## C. Tests (`crates/libs/protocol/rusty_bbp/tests`)

All (c) test fixtures: each becomes a test-local roster (`common::four_role_roster()`), not an engine concept. Line counts of occurrences: `common/mod.rs` 10 (`pid`, `vendor`, `Role::ALL` loop), `traces.rs` 59, `rules.rs` 64, `proving.rs` 25, `durable.rs` 9, `model.rs` 9 (proptest strategy `Just(Role::..)` x4), `boundary.rs` 7, `codec.rs` 0. Fixtures keep the names `planner/coder/tester/reviewer` as plain `RoleId` strings so the 31 trace tests read unchanged. `TRACES.md` (13 occurrences, prose): class c, kept; a header line says the names refer to the test roster.

## D. Host consumer (`crates/apps/rusty_bbp_host`) - not orch, but compile-coupled

| # | Location | Current text | Class | Proposed replacement |
| --- | --- | --- | --- | --- |
| H1 | src/moderator.rs:31-65 | `Launchers { planner, coder, tester, reviewer }`, `for_role` match | b | `HashMap<RoleId, Launcher>`; launcher config is per-agent, so it belongs with the orch roster. |
| H2 | src/moderator.rs:432-449 | test loops over four roles; "only the Coder keeps built-in tools" | b | Test-local roster; the built-in-tools rule is a per-agent launch policy (orch). |
| H3 | src/admin.rs:8-23 | `parse_role`/`role_name`: four-way match | a | Parse against the roster; no table. |
| H4 | src/mcp.rs:123 | tool text "request_decision from the Planner: open the plan gate" | a | "from a role that may write the spec". |
| H5 | docs/proving/agents.json, README.md, docs/proving-run.md, docs/proving/*.md | four-role launchers and run record | c | Stay as the example orch config; labelled as such. |
| H6 | tests/*_e2e.rs (112 occurrences) | four-role fixtures | c | Test-local roster. |

**Consequence for the PR split.** Dropping the enum breaks `rusty_bbp_host` at compile time, so the engine PR and the host port must land together (or the host PR stacks on the engine PR and CI is red in between). Proposed split: PR1 audit, PR2 spec v0.5.0, PR3 engine + tests + host port.

## E. Proposed roster validation (illegal states unrepresentable)

`Roster::new(Vec<Role>) -> Result<Roster, RosterError>`: non-empty; unique ids; at least one `final_decision` role and it holds `verdict_at` containing `review`; every state is servable (some role can write `spec`, write `candidate`, judge `test`, judge `review`), otherwise a task can enter a state nobody can leave. Roles with no write capability are valid (observers).

## Decisions I need from you

1. **Servability check** (last bullet of E): the brief named only the finalizer. I recommend all four checks, 4 lines of code, prevents stuck tasks. Say if you want finalizer-only.
2. **`final_decision` semantics**: at least one such `approve` (matches today's single Reviewer), versus all final roles. I recommend at least one.
3. **Roster in `task_opened`** (log format change above).
4. **Human `ask` reach**: spec table said Coder/Tester/Planner per state but the engine already lets a human address any assigned role; v0.5.0 will state the engine behavior.

## Follow-ups for orch (not touched here)

- Own the roster definitions (the four-role set becomes an orch config) and per-agent rules: read-only agents, final authority, fresh session per candidate (S4, S17).
- Own launcher config per role (H1, H2) and prompts that name roles.
- Hand the roster to BBP at `task_opened`.

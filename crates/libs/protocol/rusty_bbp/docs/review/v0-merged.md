# BBP v0: Merged Review (Claude, ChatGPT, Gemini)

All three reviews return BUILD WITH CHANGES. All were static reviews of the same text with no implementation and no access to the earlier lifecycle design or the diagrams. Per the draft's own merge rule, findings raised independently by several reviewers come first.

## 1. Raised by all three

| Topic | Agreed fix |
| --- | --- |
| Server-side cursor loses messages on crash or lost response | Stateless reads: `read(task, after: msg_id, limit)`. Per-task monotonic sequence ids. Promise at-least-once; consumers dedupe. |
| Evidence is reference-backed, not evidence-backed (R3, Q6) | `test_report` is a typed artifact (status, suite hash, revision, command) produced only by a trusted runner principal. A `verdict` must cite a `test_report` from the current iteration. Relevance stays with review. |
| Who applies the diff and runs tests is unspecified | Add the runner principal to the architecture. It applies the candidate's diff in an isolated working tree, runs the suite, and is the sole author of `test_report`. Agents never touch the shared tree. |
| Reviewer isolation is weaker than claimed | Per-role per-kind write capability. Fresh session per role is a host requirement, stated as such. Artifact bodies are untrusted data. Cut `note`. |
| Reject unknown `v` (Q9) | Integer, hard reject. |
| Keep content addressing (Q7) | Yes. Claude and ChatGPT add: separate deduplicated blobs (full hash) from task-scoped artifact records that carry `task`, `author`, `kind` and the access policy. |
| Reviewer reads no messages (Q2) | Keep. Reviewer starts from the task card and a declared input manifest (see candidate binding). |

## 2. Raised by two

| Topic | Who | Agreed fix |
| --- | --- | --- |
| Token budget not store-enforceable (R11) | Claude, ChatGPT | Store budgets bytes and message counts. Tokens are reserved and settled by the execution host, including failed attempts, and labeled advisory until that host exists. |
| Fragment resolution in R4 makes the core a parser and causes rejection loops | Claude, Gemini | v0 validates the artifact id only. Fragments are opaque client anchors, or at most `#L<start>-L<end>`. |
| Control events are not representable; states only in an image | Claude, ChatGPT | Fourth append-only record, `event`, for assignment, approval, transition, escalation, resume. Task card is a fold over events. Publish the state × kind matrix. |
| Concurrent writers and budget races | ChatGPT, Gemini | One transaction covers authorization, expected task revision, append and accounting. Caller operation id makes retries idempotent. Decide explicitly: turn token held by the moderator, or optimistic version key on the task card. Recommendation: moderator turn token in v0, since it already picks the next speaker. |
| Verdict body vs R8 (Q4) | Claude, ChatGPT | Typed `body` enum; 1,200 applies to prose fields only. Gemini's "retain as is" is compatible once the cap excludes typed payloads. |
| Documenter cannot read answers; posting tables conflict | Claude, ChatGPT | One authoritative capability matrix. Drop Documenter and Triage from v0. |
| Large artifact reads can exhaust context | ChatGPT, Gemini | `get_artifact` takes a byte or line range and returns `more`. Per-task read budget. |
| Fewer crates, defer AG-UI and A2A | Claude, ChatGPT | Core + one SQLite host + MCP. Split only when a boundary earns it. |
| Merge kinds (Q1) | Claude, Gemini | Fold `object` into `finding` with `reply_to`. Gemini goes further (5 to 6 kinds) for tool-selection accuracy. ChatGPT: keep nine. Settle after the state × kind matrix exists. |
| Cross-model decisions (Q3) | Claude, Gemini | Keep for non-gate decisions: an agent `decision` must ref an `approve` verdict from a different vendor. Never satisfies a human gate. ChatGPT dissents (below). |
| Moderator-only transitions (Q5) | Claude, Gemini | One deterministic transition function in the moderator. ChatGPT agrees, adding that verdicts may trigger it automatically, which is moderator policy and compatible. |

## 3. Raised by one, accepted

- **Candidate binding** (ChatGPT). Blocking. Verdicts and approvals float free of the revision they judge, so a Tester approve on A combines with a Reviewer approve on B. Fix: an immutable candidate manifest artifact (spec id, base and head, diff id, test evidence ids). Every `verdict` and `human_approval` refs one candidate; a new candidate invalidates prior approvals. This is also the Reviewer's input manifest.
- **Attempt identity and cancellation** (ChatGPT). Minimum: attempt generation on the task card, deadline per attempt, stale-generation writes rejected. Working-tree isolation belongs to the runner.
- **Rename the cost goal** (ChatGPT). "No double payment" overclaims. Use "reduce repeated context transfer" and measure tokens, dollars, latency, success, interventions and rejection loops against the relay baseline.
- **Soften the prior-art claim** (ChatGPT). MCP resources and subscriptions could serve the read path; A2A has tasks, artifacts and lifecycle. BBP's contribution is shared-task policy, not transport.
- **orch-core overlap** (Claude). Neither other reviewer could see the ecosystem. First decision to make: BBP as the protocol over orch-core's Board, or `bbp-core` as orch-core's next version, never a parallel crate.
- **`supersedes` field** (Claude). Retraction without deletion. Also answers Gemini's "object references a superseded proposal" scenario.
- **`msg:` refs** (Claude). Needed so a `decision` or verdict can cite a message, and for candidate binding.
- **Evidence from the current iteration only** (Gemini). Cheap, mechanical, closes the stale-log stuffing case.

## 4. Disagreements

- **Cross-model decisions (R7, Q3).** ChatGPT: human-only in v0, model agreement is not authorization. Claude and Gemini: allow for non-gate decisions with a different-vendor approve verdict, which is the rusty_orch decision policy Nano set on 2026-10-01. All three agree gates are human-only (R13). Recommendation: keep the policy, scope it in text to non-gate decisions. Nano decides.
- **Kind count (Q1).** Two for merging, one against. Low stakes; defer to the matrix.

## 5. Settled answers to Q1–Q10

- Q1: lean merge; settle after the state × kind matrix.
- Q2: no messages; Reviewer starts from the candidate manifest.
- Q3: non-gate only, different-vendor approve verdict, pending Nano's call.
- Q4: typed bodies; 1,200 on prose fields.
- Q5: moderator owns one deterministic transition function; gates stay human.
- Q6: typed `test_report` from the runner, current iteration, candidate-bound; relevance is review's job.
- Q7: yes, blobs plus records.
- Q8: host-issued principal, task membership, role and attempt generation. One MCP server process per role so role is not a field.
- Q9: reject unknown.
- Q10: runner and working-tree mediation, candidate manifest, event log, idempotency, attempt and cancellation, artifact range reads, who performs the approved merge.

## 6. Order of work for the next draft

1. Decide the orch-core relationship.
2. Add candidate manifest, runner principal, event record, attempt generation, operation id. Rewrite R3, R4, R7, R11 accordingly.
3. Publish the state × kind matrix and one capability matrix.
4. Prove one two-agent task survives a lost response, a crash and a stale approval before adding roles or adapters.

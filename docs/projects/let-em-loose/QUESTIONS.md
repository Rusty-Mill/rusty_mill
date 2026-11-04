# Let Em Loose: pre-answered questions

These are the questions an implementer would ask while building v0 ([SPEC.md](SPEC.md)), with their answers. Most were settled during the #592–#597 design reviews on 2026-11-02; each settled answer names the decision in [DECISIONS.md](DECISIONS.md) that records it. Answers marked **[Nano decides]** are still open. Read the [authority/provenance limits and merged contract links](DECISIONS.md#authority-provenance-and-current-contracts) before using these summaries; reported approval and planning ceilings are not execution or spending permission.

## Still open

| Q | Decision needed |
| --- | --- |
| 28 | Signing off the golden sets |

Q26, Q27 and Q31 were settled after this list was written; see [handoff/QA.md](handoff/QA.md) (§ Dedup experiment, § remind-me).

## Claims model (BBP, #593)

**1. What exactly is the confidence update rule?**
A whole-number score. An executed test adds +4 for a pass and -8 for a failure, with strength fixed at 1. A cited source counts 2. Opinion alone can never reach accept. Track records are recorded but not used as weights in v0. (D29)

**2. What are the accept and drop lines?**
Accept at 8 (90%) for low and medium stakes, and 12 (about 96%) for high-stakes and irreversible claims. Drop at -8 (10%). At least two distinct authors are needed to accept. Irreversible claims always escalate, whatever their score. (D29)

**3. What happens to dependents when a dependency collapses?**
Dependents nothing has acted on reopen automatically; if the dependency was dropped, they are voided in the same step and stay visible. Only dependents already acted on escalate. A new claim cannot depend on a dropped one. (D32)

**4. How are "settled" and "stuck" detected?**
By evidence posts, not rounds. If the last 6 posts moved the score by 2 or less in total and the claim is still between the lines, it escalates as stuck. Settled claims keep taking evidence: an executed contrary result reopens them into a fresh epoch. (D28, D29)

**5. How do existing BBP tasks and posts relate to claims?**
Claims sit beside the stage lifecycle as a per-task mode; stage-mode tasks are unchanged. Evidence posts need no turn; state changes, budgets and approvals keep the one-live-turn rule. Old logs are read as they are, with no migration. (D27)

**6. Can claims be edited?**
No. Claims are immutable and evidence is append-only. A change is a new claim that supersedes the old one, and it takes effect only once the new claim is accepted. Superseding an acted-on claim needs Nano's accept. (D31)

**7. Who can create claims?**
The seeding pass, any lens, and Nano through intent. Storing a candidate that names a claim starts its run. `sole_candidate` is recorded on the claim; orch decides what to do about it. (D29)

**8. How are concurrent writes handled, and what does "deterministic" promise?**
Evidence is an append-only log. The promise is that the same log always replays to the same outcome; settling depends on arrival order, which the log records. Before acting on a claim, the host records an action mark against the epoch it saw. (D28, D33)

## Lenses and orch (#594)

**9. Which executor does the lens runner extend?**
orch, next to the dispatcher, on its existing model port. super-board was retired on 2026-10-28, and mill-runner is left alone. (D20)

**10. What are the starter lenses?**
Three blind Architect proposers per question, then Research, Adversary and Operator. The holdout lens comes with #597. Builders arrive with #600. (D34)

**11. Which model runs each lens?**
Architects on Claude, local and Codex, with the rebel set in config. Operator on Qwen3.6-35B-A3B via llama-swap. gpt-oss-120b is the local high tier for sensitive or contested claims. Context starts at 32k. No Gemini in v0. (D34)

**12. What does the port contract look like?**
Request: lens id, question and the assembled context. Response: evidence items or new claims, checked against a schema in the adapter, with non-conforming replies rejected. The hard-coded provider list becomes a config registry; `orch-openai` is a new adapter crate for llama-swap, allowing plain HTTP only to loopback and private addresses. (D34)

**13. How is the blind first pass enforced?**
Context assembly excludes the other lenses' evidence on a first pass, and the same barrier must cover subsequent MCP/tool reads. The claims addendum's **Reads, delta and subscriptions** requires session credentials, message visibility on `read`, `claims_delta`, `claim_card` and `claim_line`, nested-content filtering and artifact permissions on `get_artifact`. A blind tool participant must use an equivalently restricted/scoped session or have those tools withheld for the blind pass; a `read_messages: all` session with a filtered prompt is insufficient. Host/human unfiltered views must not be exposed as agent tools. Each question gets three blind Architect proposals before any attack. (D7, D34; [precise contracts](DECISIONS.md#authority-provenance-and-current-contracts))

**14. How is "agreement costs evidence" enforced?**
Executed support must cite a final, passed, runner-authenticated `test_report`, bound through its run secret to its selected run, candidate, claim and frozen profile digest; superseded run secrets are refused. C-R4 and **Runs and candidates** in the claims addendum define this binding; a resolvable reference or builder's assertion alone is not executable evidence. Agents cannot write runner reports/logs. Unverified builder results remain opinion, which cannot reach accept. Cited sources use `source` artifacts and retain cited weight, not executed weight. Holdout acceptance additionally binds the suite digest and current candidate's actual tree (D30, D37).

**15. How is "look harder" triggered?**
A critic lens that finds nothing gets one "look harder" retry; a second empty pass is accepted and recorded.

**16. How are track records handled in v0?**
Recorded and reported, not used as weights. (D29, D37)

**17. How do builders take part?**
Through #600's builder, in a sandboxed worktree, posting candidate runs as executed evidence. Joining the board over MCP (D14) is still proposed.

**18. Where do lens prompts come from?**
From the generic experts in Expert-Agents, bound and versioned as config in the Factory repo.

## Budget and cost

**19. What are the budget units and caps?**
One unit buys one evidence post or one candidate run. Claims get 8, 16 or 32 units for low, medium or high stakes, doubled if irreversible. Phase B planning ceilings are $200 total, $25 per run, and a pause if the first paired intent exceeds $40, pending separate spending authorization. Neither these figures nor a BBP extension authorizes billed calls: v0 refuses nonzero rates and requires operator-controlled subscription/host admission. (D29, D38)

**20. Who assigns stakes and reversibility?**
The intent declares never-rules and reversibility classes, and the seeding pass tags each claim. Anything that writes to user data, shared branches or external systems defaults to irreversible.

## Inbox and Nano (#595)

**21. Where does the inbox live?**
A read view over BBP in Mill Board, reading only through the host's `bbp` command. Nothing goes into rusty_tick. (D35; this reverses the earlier recommendation.)

**22. How is Nano notified?**
No push notifications in v0. (D35)

**23. What do the actions do?**
D35 and the merged inbox's `plan`/`plan_task` define the per-reason actions. Claim-budget Approve extends one claim base allowance and reopens, preserving the failure block; held-rerun Approve extends and starts the rerun. Whole-task Approve extends each exhausted total by its own opened allowance, then resumes, without accepting a claim. Explicit human `accept`/`choose` is the separate threshold/failure override; irreversible Approve adds no budget. Reject drops (or cancels a whole task with a reason), with remediation for acted-on claims. Choose selects among a stuck question's candidates. Refine records the clarification and reopens a claim, or extends exhausted totals and resumes a paused task. Preview and execution are revision-fenced against the displayed snapshot, not generic confidence evidence or permission for future actions; a stale revision requires re-reading. See D35's [precise contracts](DECISIONS.md#authority-provenance-and-current-contracts), including the action-mark fence and the current intent-versioning limit.

**24. What counts as irreversible?**
Deleting, moving or overwriting user data. Pushing to shared branches. External messages. Schema migrations. Spend above a tier allowance. Agents never merge PRs in any case.

## Intent

**25. What is the intent format?**
YAML with `id`, `goal`, `interface`, `acceptance`, `non_goals`, `taste_domains` and `oracle`, kept in the Factory repo. Acceptance lines describe observable behaviour, never method. (D38)

**26. Who runs the intent interview for #600?** **[Nano decides when]**
Claude, in chat with Nano, interviewing by example. The output is committed to the experiment repo.

## Experiments

**27. What are the targets?** **[Nano decides for #600]**
Phase B is settled: five small intents, a pre-registered decision rule, and a `dedup-lite` timing target of 2 seconds on Nano's desktop, warm cache, median of 5. The #600 experiment, the full 20 TB app, gets its targets from the intent interview. (D36, D38)

**28. What ground truth is used?** **[Nano signs off the golden sets]**
Holdout suites written from intent only, plus golden sets in the Factory repo with Nano's human digest-attestation. The host verifies manifest hashes and the exact SIGNOFF digest, not cryptographic signer identity; CODEOWNERS/file presence does not prove authorship (D37). Private fixtures from Nano's drive are recorded as hashes and mounted read-only. (D37)

**29. What may the agents read?**
Public prior art, including fclones, for study only, with no copied code. No access to `rusty_fclone` or rusty_mill. Builders see only pass or fail counts for holdouts.

## Process

**30. Where does work land, and how?**
BBP rules in the claims addendum (PR #1386). Each card's design on its card. Code in the existing crates, one PR per card, test-first, with a change fragment, reviewed by `dottiusmaximus`. Agents never queue or merge. The original next step was the #593 engine; check current main and per-card approvals for current work. This historical guide does not authorize an implementation start. (D40)

**31. What about existing remind-me records?** **[Nano decides]**
The explicit `project` parameter is approved. Any write to existing records waits until Nano has seen exactly which records it would change. (D39)

## Erosion gate (#704)

**32. What happens when report-only expires?**
At 2 weeks or 10 tasks, whichever comes first, pause new affected tasks until Nano approves thresholds. Existing tasks retain their settings and frozen profile digests. Calibration is reported for approval; no guessed thresholds or indefinite report-only extension. (D42)

**33. How does cleanup work at zero?**
Require at least one metric to improve and neither to worsen. Skip cleanup if both baseline metrics are zero; log the reason. For example, (100, 0) to (80, 0) passes, unchanged (100, 0) fails, and (0, 0) skips. D44 gives the full predicate and examples. These owner choices are recorded in the linked D42/D44 decision record.

# Let Em Loose: pre-answered questions

These are the questions an implementer would ask while building v0 ([SPEC.md](SPEC.md)), each with a proposed answer. Answers marked **[Nano decides]** are recommendations only: they need Nano's call before code depends on them. Everything else is a default the implementer may adopt, recording any deviation in [DECISIONS.md](DECISIONS.md).

## Nano's open calls at a glance

| Q | Decision needed |
| --- | --- |
| 2 | Default accept and drop thresholds |
| 5 | BBP schema version bump for claims |
| 9 | Which executor the lens runner extends or replaces |
| 11 | Starting model for each lens, given API cost |
| 19 | Global spend ceiling per run |
| 21 | Whether the inbox lives in Mill Board or rusty_tick |
| 22 | How Nano is notified of escalations |
| 26 | When the intent interview happens |
| 27 | Target scan time and reference hardware |
| 28 | Signing off the golden set |

## Claims model (BBP, #593)

**1. What exactly is the confidence update rule?**
Log-odds accumulation. A claim starts at 0.5 (log-odds 0). Each evidence item adds `direction × strength × kind_weight × author_weight` to the log-odds, and confidence is the sigmoid of the total. Starting kind weights: executable test or benchmark 1.0, measurement 0.8, sourced citation 0.5, opinion 0.0 for support and 0.3 for counter-evidence, so objections stay cheap and agreement doesn't. `author_weight` is 1.0 in v0 (see Q16). It must be a pure function of the evidence log and covered by trace tests. The weights are tuned later through calibration (#597).

**2. Are the thresholds fixed or per claim?** **[Nano decides]**
Recommended: accept at 0.90 and drop at 0.10 by default, overridable per intent. A claim marked irreversible needs 0.95 and always escalates, whatever its confidence.

**3. What happens to dependents when a foundation claim collapses?**
They move to `reopened` and are flagged, not deleted. Their confidence is capped at their parent's until the parent recovers or they are re-proven on other grounds. All history is kept.

**4. How is "settled" detected?**
A claim is settled when its last 3 evidence events each moved confidence by less than 0.02 and it sits outside the open band. Both numbers are config. A settled claim stops attracting work and is compacted to one line plus an evidence link.

**5. How do existing BBP posts map onto claims?** **[Nano decides]**
Keep the post as the transport and add `claim` and `evidence` payload types. Today's free-form posts become opinion-kind evidence or are left untyped. This needs a schema version bump. Recommended: an additive bump with a reader for old logs, and no migration of historical logs.

**6. Can claims be edited?**
No. Claim text is immutable, and evidence is append-only. A change creates a new claim that `supersedes` the old one. This makes illegal states unrepresentable and keeps replay exact.

**7. Who can create claims, and how are duplicates handled?**
The seeding pass, any lens, and Nano through intent can all create claims. The board flags near-duplicates, and merging them is an explicit `supersedes`, never silent.

**8. How are concurrent writes handled?**
Evidence is an append-only log, so there are no write conflicts. Confidence is recomputed from the log. State transitions, budget debits and escalations go through the serialized control plane. Confirm on main what Phase 2a already provides before designing (#593 first step).

## Lenses and orch (#594)

**9. Which executor does the lens runner extend or replace?** **[Nano decides]**
Decide after the reconnaissance report. Leaning: extend the orch dispatcher for lenses, and reuse mill-runner's worktree and rusty_sandbox for builders. Retire nothing without Nano's approval.

**10. What are the starter lenses?**
Adversary, Operator and Research, plus a Builder that is not a critic. Holdout and Intent lenses come with #597.

**11. Which model runs each lens at first?** **[Nano decides]**
Recommended: Builder on Claude, Adversary on OpenAI/Codex, Research on Gemini, Operator on the local llama-swap model. Every lens falls back to the local model when its budget runs low. This depends on API keys and cost.

**12. What does the port contract look like?**
Request: lens id, question, and the assembled context bundle. Response: a JSON list of evidence items or new claims, checked against a schema in the adapter. Anything that doesn't conform is rejected, retried once, then logged as a lens failure. Adapters return `Result`, never panic, and attach the provider and model to every item.

**13. How is the blind first pass enforced?**
Through context assembly. For a claim in its candidate phase, a lens's bundle excludes other lenses' evidence on that claim until every asked lens has posted or timed out. It is enforced by construction, not by the prompt.

**14. How is "agreement costs evidence" enforced?**
Support evidence of kind opinion has zero weight (Q1). Support counts only if it references an artefact the board can resolve: a test id, a benchmark run or a citation URL.

**15. How is "look harder" triggered?**
When a critic lens posts no counter-evidence on 3 consecutive contested claims, it is re-asked once with an explicit adversarial framing. A second empty pass is accepted and recorded.

**16. How are track records handled in v0?**
Logged only. Each lens's evidence is recorded against final outcomes, but `author_weight` stays at 1.0 until there is enough data. Difficulty-weighted scoring comes in #599.

**17. How do builders (Claude Code, Codex) take part in v0?**
They work in a sandboxed worktree through the MCP door to the board (D14), posting test and benchmark results as executable evidence. Exactly how depends on Q9.

**18. Where do lens prompts come from?**
From the generic experts in Expert-Agents, bound and versioned as config in the Factory repo. orch loads them by name and version, and nothing is hard-coded.

## Budget and cost

**19. What are the budget units and starting numbers?** **[Nano decides]**
Tokens and wall-clock time per claim, by tier: low, medium, high and irreversible, each tier with roughly four times the allowance of the one below. Nano sets the global ceiling per run, in dollars or tokens. The circuit breaker pauses the run and escalates when spend rises while total confidence movement stays flat (#598).

**20. Who assigns stakes and reversibility?**
The intent declares never-rules and reversibility classes, and the seeding pass tags each claim. Anything that writes to user data, shared branches or external systems defaults to irreversible.

## Inbox and Nano

**21. Where does the inbox live?** **[Nano decides]**
Recommended: store escalations in rusty_tick and show them in Mill Board, matching the 2026-10-26 decision that Mill Board is the agent view over rusty_tick. #595 confirms.

**22. How is Nano notified?** **[Nano decides]**
For v0, recommended: the inbox only, with no pings. Push notifications can come later.

**23. What do Approve, Reject and Refine do?**
Approve posts human evidence that is decisive, and the action proceeds. Reject drops the claim with Nano's reason attached. Refine opens the intent: Nano's edit bumps the intent version and reopens the claims bound to the old version (#598).

**24. What counts as irreversible?**
Deleting, moving or overwriting user data. Pushing to shared branches. External messages. Schema migrations. Spend above a tier allowance. Agents never merge PRs in any case (repository rule).

## Intent

**25. What is the intent format?**
A Markdown file with a YAML block: `goal`, `targets`, `always`, `never`, `examples`, `taste`, `reversibility`, plus a `version`. The template lives in the Factory repo, and each project keeps its own copy. Every never-rule should map to at least one property test.

**26. Who runs the intent interview?** **[Nano decides when]**
Claude, in chat with Nano, interviewing by example: concrete cases rather than requirement questions. The output is committed to the experiment repo as `INTENT.md`.

## The dedup experiment

**27. What are the success criteria?** **[Nano decides the targets]**
Compared with `rusty_fclone` on the same data: scan throughput, work lost on a crash, zero false duplicates on the golden set, GUI usability as judged by Nano, tokens or dollars spent, and Nano's minutes. Nano sets "acceptable scan time" and the reference hardware.

**28. What test data is used?** **[Nano signs off the golden set]**
A synthetic generator that reproduces the shape of the 20 TB drive (file-count and size distribution, nesting, duplicate ratio) at a smaller scale, plus a golden set of real cases with known answers. The real drive is used only in a read-only shadow run.

**29. What may the agents read?**
Public prior art, including fclones, papers and benchmarks, for study only, with no copied code. No access to `rusty_fclone` or rusty_mill. Network access follows the sandbox policy.

## Process

**30. Where does work land, and how?**
Designs go in `docs/projects/let-em-loose/` and as comments on the cards. Code goes in the existing crates. One PR per card, each with a change fragment, reviewed by `dottiusmaximus`. Agents never queue or merge. Decisions are saved to remind-me with the tag `let-em-loose`, the card number, and `project="let-em-loose"`.

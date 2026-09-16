*Point-in-time review (AI engineer lens), 2026-05-27. Superseded by the canonical PRDs once they absorb these edits — do not cite as spec.*

# AI Engineer Review — Rusty Keys

## 1. Scope & lens
The model-facing intelligence: system-prompt construction, per-turn context assembly, recall scoring, embeddings, consolidation contracts, criteria judge, model selection, and the eval plan that turns M-HIR + the 5-label taxonomy into live metrics. PRDs name most of these mechanisms but leave the *algorithms* and *prompts* unpinned — the implementer would have to invent them, and divergent invention is how the research thesis quietly dies.

## 2. Validated gaps (the focus list holds)
1. **System-prompt construction is entirely unspecified.** `Kernel.system_prompt: String` (01:67) and `system(system_prompt)` (01:20) are inputs with no producer. No PRD says what goes in it: role/identity, tool-use protocol, H3 reproduce→attribute→fix discipline, safety rails, or *how* it varies by harness level. This is the single largest hole for the AI lens.
2. **Task-State injection mechanics undefined.** 03:331 says the harness "injects the active task into the system prompt (drift prevention)" but `task_store.render()` (03:340) has no body — yet 03:346 *also* says the oriented string is injected "between the system prompt and the first user message." So Task State is claimed to live in two places. Pick one (or specify both deliberately) and define the rendered shape.
3. **Recall scoring formula is named, never pinned.** 03:270 "relevance + recency + importance"; no weights, no decay function, no normalization across the lexical-vs-cosine relevance domains. `candidates()` returns raw `f32` relevance (03:254) whose scale differs between FTS5 and cosine — combining with recency/importance without normalizing is undefined behavior.
4. **Recall output string format undefined.** `recall(...) -> Result<String>` (03:273) — what does the assembled block look like? Headers, per-memory framing, neighbor expansion layout, token cap? The model's orientation quality depends entirely on this and it's a black box.
5. **Embedding strategy is a single env var.** `RUSTYKEYS_EMBED_MODEL` (06:301) with no dimensions, chunking rule, similarity threshold for the candidate cutoff, or how lexical fallback blends when *some* memories lack embeddings (mixed corpus during migration). DuckDB `list_cosine_similarity` (03:263) named; no schema/index detail.
6. **Consolidation output contract is one line.** 03:300 "JSON output `{"memories": [...]}`" — no `Memory` field schema for the model to emit, no importance-assignment rubric, no dedup/merge-vs-create rule, no quality gate beyond "serde_json deserialises." The three tempos (03:294) differ only in trigger; idle vs sleep *prompt* differences are unspecified.
7. **Criteria judge has a prompt but no robustness spec.** Prompt exists (05:148-166) — good. But: single model call, no handling of nondeterminism (run-to-run flips on borderline criteria), no self-consistency/retry, and graceful degradation *passes* on parse failure (05:191) — a silent false-positive that inflates "verified." That failure mode should at minimum be journaled and ideally not count as verified.
8. **Model selection is one knob for four jobs.** `RUSTYKEYS_MODEL` (06:291) drives kernel, criteria judge (`CriteriaJudge.model` 05:138 — separate field, never configured), consolidation, *and* compaction summaries (06:98). No guidance that judge/consolidation/embeddings could/should use cheaper-faster models. Cost/latency story absent.
9. **No eval/benchmark plan exists.** `docs/dev/` does not exist. M-HIR (04) and the 5-label taxonomy (05:237) are computed live but there is no doc defining how to *use* them as maturity signals over time, no golden-episode regression suite, no H1→H2→H3 progression gates. BACKLOG phases are tagged H1/H2/H3 (BACKLOG:10-66) but "done" has no measurable exit criteria. This is the doc that operationalizes the paper — must-have.

## 3. Already-covered / pruned
- **Criteria-judge prompt & per-criterion contract**: covered — `05-compose.md:148-166` (prompt + `{"verdict","criteria":[{criterion,met,reason}]}`). Gap is robustness, not existence.
- **Recall query from a window, not last message**: covered — `03-feed.md:282-285` (good drift-prevention rationale, keep).
- **Lexical fallback exists**: covered — `03-feed.md:286-289`, `06-app.md:301`. Gap is the *blend/threshold*, not the fallback.
- **3-tier compaction**: fully specified — `06-app.md:93-103`. My only add is the *interplay* with recall/system-prompt token accounting (see New gap N3).
- **Consolidation tempos & verification-signal weighting**: covered — `03-feed.md:294-308`. Gap is the JSON/prompt contract.
- **Outcome taxonomy enum & triggers**: covered — `05-compose.md:237-253`. UnsafeInvalid trigger pinned. Keep.
- **Post-turn concurrency / join-before-observe ordering**: covered — `05:178-188`, ADR-012. Not re-litigating.

## 4. New gaps (found while reading)
- **N1 — Context ordering & precedence is unstated.** Final prompt = system + `extra_context` (orient = task_prompt + recall, 03:339-343) + history. But after micro-compact drops turn-pairs and session-summary inserts `[SUMMARY]` (06:98), what is the canonical message order? Recall pulls from long-term, history holds short-term, compaction summarizes history — these can *restate the same facts three ways*. No de-duplication / precedence rule between recalled memory and in-window history.
- **N2 — Recall ignores the entropy/verification learning loop at assembly time.** Consolidation consumes verification signals (03:305-308), but recall scoring (03:270) doesn't privilege skills born from UNVERIFIED failures when the current turn resembles the failure context. The "don't repeat the mistake" path depends on it surfacing — needs an importance/`type=skill` boost in scoring.
- **N3 — Token budget has no line items.** `TokenBudget` (06:81) tracks totals but the per-turn assembly (system prompt + recall block + task render + tool schemas + history) has no budget split. Recall `k=6` (06:302) and tool schemas (~26 tools + MCP) consume fixed overhead that isn't accounted before the 80/90/95% compaction triggers fire on *history alone*.
- **N4 — `agent` subagent system prompt undefined.** Subagents (03:118-127) inherit Config but get isolated history. Do they inherit the parent system prompt, get a focused-subtask variant, or carry Task State? Unspecified, and it shapes subagent reliability.
- **N5 — Consolidation/judge model-failure isn't a metric.** Parse failures degrade silently (05:191; 03 implies similar). These are themselves harness gaps and arguably belong in M-HIR-adjacent telemetry, or at least the eval plan's health checks.

## 5. Recommended edits

| target file | change | priority | depends-on |
|---|---|---|---|
| 03-feed.md (+ new `docs/dev/prompts.md`) | Add **System-prompt construction** section: producer, layered template (identity → tool-use protocol → harness-level rules → safety), and how it composes with `extra_context`. Resolve the "Task State in system prompt vs between" contradiction. | **P0** | constrain (safety rails), kernel (consumes it) |
| 03-feed.md | Pin the **recall scoring formula**: weights, decay fn, score normalization across lexical/cosine, neighbor-expansion rule, output-string format, token cap. (v1 intent, revisit after a spike) | **P0** | embeddings decision (N below) |
| docs/dev/eval-plan.md (new) | Author the **eval plan**: live M-HIR + 5-label dashboards, golden-episode regression suite, H1→H2→H3 progression gates with measurable exit criteria, judge/consolidation health metrics. | **P0** | systems-architect (storage of golden episodes), product (gate thresholds) |
| 03-feed.md | Define the **consolidation JSON contract**: full `Memory` emit schema, importance rubric, create-vs-merge rule, idle-vs-sleep prompt deltas, dedup quality gate. (v1 intent, revisit after a spike) | **P1** | recall formula (importance scale must match) |
| 05-compose.md | Harden **CriteriaJudge**: parse-failure must NOT silently pass-as-verified (journal it, mark `judge_unavailable`); add optional self-consistency (n=2-3 majority) for borderline criteria. | **P1** | eval-plan (defines acceptable nondeterminism) |
| 06-app.md / 03-feed.md | Add **per-turn token budget line items** (system + recall + task + tool schemas + history) feeding the compaction thresholds; specify recall+history de-dup precedence (N1/N3). | **P1** | compaction tiers (06:93) |
| 06-app.md (Config) | Add **per-role model knobs** (`RUSTYKEYS_JUDGE_MODEL`, `RUSTYKEYS_CONSOLIDATE_MODEL`, `RUSTYKEYS_COMPACT_MODEL`) + cost/latency guidance; wire `CriteriaJudge.model`. | **P2** | — |
| 03-feed.md / 06-app.md | Pin **embedding strategy**: model family, dims, chunking, candidate similarity threshold, mixed-corpus fallback blend, DuckDB schema. (Phase 5; v1 intent, revisit after a spike) | **P2** | systems-architect (DuckDB), recall formula |
| 03-feed.md | Specify **subagent system prompt / Task-State inheritance** (N4). | **P2** | system-prompt section |

### v1 sketches (mark all algorithmic specifics: v1 intent, revisit after a spike)

**Recall score** (per candidate, after min-max normalizing relevance within the returned batch so FTS5 and cosine share a [0,1] domain):
```
score = 0.55*rel_norm + 0.25*recency + 0.20*importance
recency = exp(-Δdays / 14)          # 2-week half-life-ish; τ tunable
importance = stored 0..1 (skills floored at 0.6)
# tie-break: type=skill > summary > fact; then most-recent.
# take top-k (k=6), then 1-hop neighbors() of the top-3 only (cap added tokens).
```
Output block (the string `recall()` returns), capped at a configured token slice:
```
## Relevant memory
- [skill] <title>: <body>            (why: matched "<query frag>")
- [fact]  <title>: <body>
  ↳ related: <neighbor title>
```

**Consolidation contract** — model emits:
```json
{"memories":[{"op":"create|update","type":"fact|summary|skill|entity",
  "title":"...","body":"...","importance":0.0-1.0,
  "edges":[{"to":"<title>","rel":"relates|causes|supersedes"}],
  "source_ts_range":[t0,t1]}]}
```
Rubric in prompt: verification=UNVERIFIED → emit one `skill`, importance ≥0.8, body = the lesson + the failing condition. Dedup: if `title` cosine/lexical-matches an existing memory > threshold, emit `update` not `create`. Idle prompt = "extract only NEW durable facts/skills from the last N observations"; sleep prompt adds "merge near-duplicates, decay stale importances."

**Criteria judge** — keep 05:148 prompt; add: on parse/call failure, return `passed=true, detail="judge_unavailable"` **but set a `judge_ran=false` flag** so the journal records it and `outcome` cannot be `AutonomousVerifiedSuccess`. Optional self-consistency for criteria whose `met` flips across 2 samples → mark `met=false` (strict).

**Eval plan** (skeleton):
1. *Live metrics*: M-HIR trend (per session), outcome-label histogram, judge-unavailable rate, entropy delta cumulative, recall hit-rate proxy.
2. *Golden episodes*: a fixtures dir of frozen tasks + expected `EpisodeOutcome` + `checks.toml`; replay harness asserts outcome label and no regression in deterministic checks. Stored as the same episode-package JSON (05:257) so prod and eval share one format.
3. *Progression gates* (measurable exit criteria, v1 intent): **H1**: 100% tool-call schema validity, clean-termination ≥X% on golden set. **H2**: cross-session recall surfaces the planted fact ≥X%, Task-State drift (task_override rate) below threshold. **H3**: AutonomousVerifiedSuccess ≥X% AND UnsafeInvalid = 0 on golden set, every H3 turn produces a complete 8-trace package.

## 6. Cross-persona dependencies
- **Systems architect**: recall-score storage (importance column, decay-at-read vs decay-at-write), DuckDB vector schema, golden-episode fixture storage, token-accounting plumbing in `TokenBudget`. My recall/embedding edits depend on their storage decisions.
- **Product/research owner**: the *thresholds* in the progression gates and acceptable judge-nondeterminism budget are product calls; I sketch shape, they set numbers.
- **Constrain/security persona**: system-prompt safety-rail content and whether subagents inherit reduced authority overlaps with their policy spec.
- **DX/CLI persona**: `/stats`, `/mhir`, and a likely new `/eval` surface the eval metrics; the eval-plan's live dashboards must align with what the CLI/desktop already expose (06:189-193, BACKLOG:190).

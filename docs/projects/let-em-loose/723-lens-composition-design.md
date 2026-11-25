# #723 Lens composition: design

Status: draft for Nano's approval on card #723. Nothing is built until he approves (D25).
Released by D177. Project layer per D178. Direction from D133.

## Goal

orch builds each lens's prompt **per question** from a catalog, not from one flat file per lens:
the lens prompt, the project's domain pack, and only the knowledge and standards files the
question's domains touch. The same composition yields the tool request that #1378 compiles
into gateway rules.

Done when: `lens run` builds different contexts for two claims in different domains from one
catalog, and #1378 can load its policy from the composition.

## What changes from today

Today (`rusty_orch/src/lens/run.rs`, `orch-lens/src/context.rs`) each lens has one prefix,
built once: `# Intent`, `# Board rules`, `# Your lens` (the flat `lenses/<id>.md`, ADR-0021,
D135), `# Reply`. The prefix is byte-identical across all jobs of a lens so providers cache it.

After: the prefix is keyed by **(lens, domain set)**. Same lens + same domains = same bytes,
so caching still works per domain set. With composition off, or with no domain pack and no
mapped domains, the prefix is byte-for-byte today's.

## Inputs

| Input | Source | Required |
|---|---|---|
| Lens prompt | Factory `lens.json` → `lenses/<id>.md` (D135) | yes |
| Domain pack | target repo `DOMAIN.md` (D178), a short digest | no |
| Manifest | target repo manifest written by Ai Env Manager (D178): extra knowledge topics and tools | no |
| Domain map | Factory `domains.toml` (new): domain token → knowledge and standards files | no |
| Knowledge / standards | Factory `knowledge/*.md`, `standards/*.md` | only what the map selects |
| Question domains | the claim's existing `domains` field (1–8 tokens); a question takes the union over the claims it references | yes |

No BBP change: `domains` already exists on every claim and orch already reads it (`card.rs`,
`config.rs` `Lens::follows`). Candidate diff paths are not used in v0.

The project-expert pack is not composed into prompts; it stays asked on demand (D178).
`.factory/overlay.md` is not an input.

## Domain map (`domains.toml`, in Factory)

```toml
[domain.rust]
knowledge = ["knowledge/rust-expert.md"]
standards = ["standards/rust.md"]

[domain.terminal]
knowledge = ["knowledge/terminal-emulator-expert.md"]
```

File names above are illustrative; this session could not read Factory, so slice 4 picks
the real ones. Unknown domain tokens map to nothing (logged once per run, not an error). Files are deduped and
sorted, so the order of a claim's domains never changes the bytes.

## Output

A pure value, built with no I/O:

```rust
pub struct Composed {
    pub sections: Vec<Section>,   // lens, domain pack, knowledge, standards, in that order
    pub tools: ToolRequest,       // what the lens asks for + what the manifest adds
    pub key: ComposeKey,          // (lens id, sorted domain set): the cache key
}
```

The prompt order becomes: `# Intent`, `# Board rules`, `# Your lens`, `# Project`
(DOMAIN.md), `# Knowledge`, `# Standards`, `# Reply`. Rules stay in code and are never
repeated in packs (ADR-0021), so no pack can loosen a rule.

`ToolRequest` is capped at launch by the agent profile (D160, D162): the composition can only
narrow what the profile allows, never widen it. #1378 owns the TOML policy shape and the
gateway compile; it reads `ToolRequest` and the lens id, nothing else.

## Budget

Composed sections count against the provider window before settled lines (`context.rs`
order: card is never cut). If prefix + card do not fit, sections drop in reverse order
(standards last to go, knowledge first), each drop is logged, and only then is it a
`TooLarge` refusal. Per-file cap: 8 KB (default, config).

## Standards: must and should

Standards are expressed in the prompt only, through existing posts. No new BBP field:
- a broken **must** → the lens posts a counter citing the standard in `refs`;
- a broken **should** → the lens posts a note with no counter (confidence unaffected).

The card body asked that a broken *should* lower confidence. Doing that for real needs a
severity field in BBP, which is a schema change and ask-first. Not proposed for v0.

## Where it lives

- `orch-lens/src/compose.rs`: the pure composer. Takes already-read texts, returns `Composed`.
- `rusty_orch/src/lens/catalog.rs`: the Factory catalog adapter. Reads `lens.json`,
  `domains.toml`, the selected files, and the target repo's `DOMAIN.md` and manifest.
- `lens run` switches to it behind config: `"compose": true` in the lens config, off by default.

Roles stay in orch. BBP sees nothing new: lens id = principal, as today.

## Retro and calibration

The v0 composer takes no feedback input. Retro is a later slice: it reads #599 per-lens track
records and proposes Factory PRs (domain map, knowledge files), never automatic edits.
Approving this design approves that scope, not the Retro build.

## Slices (one PR each, stacked)

1. `compose.rs` + fixture tests: same lens, two domain sets → different bytes; same set in any
   order → same bytes; no inputs → today's prefix; budget drops in order.
2. Catalog adapter + `domains.toml` reader, tested on a fixture catalog directory.
3. `lens run` behind `compose`, with the prefix cache keyed by `ComposeKey`. Test: two claims
   in different domains get different contexts from one catalog.
4. Factory PR (separate repo): `domains.toml` for the first domains. Needs Factory access in a
   session that has it.

No dependency bumps: `toml = "0.8"` is already a workspace dependency; orch-lens would add it
as a `workspace = true` crate dependency (new to orch, not a new crate or version).

## Questions for Nano

**Q48. Are Factory's workflow roles and the board lenses one set of roles or two?**
- a) One set for orch: the lens is the role (D133); Factory role files give only ground rules;
  Factory's gated pipeline stays outside orch. **Recommended**: matches D133 and the card's
  "drop the pipeline and gates", changes nothing in Factory's own pipeline.
- b) Two sets: orch composes both.
- c) Retire Factory workflow roles altogether.

**Q46. Where do knowledge and standards packs live for the composer?**
- a) Factory `knowledge/` and `standards/`, read through the catalog adapter; lenses stay in
  Factory `lenses/` (D135); DOMAIN.md stays in the target repo (D178). **Recommended**: Factory
  is the source of truth (D19), and a later move only needs a new adapter.
- b) A pack catalog in `baileyrd/pack-validation`.
- c) Copied into each repo under `.agents/packs/`.

Q47 (when the expert retires) does not block this; `let-em-loose-expert` keeps answering.

## Not in scope

The card's "drop the pipeline and gates" (step 3) and moving domain knowledge out of
Expert-Agents prompts (step 2) are Factory/Expert-Agents repo work. This design only makes orch
able to consume the result; those moves follow once Q46 and Q48 are answered.

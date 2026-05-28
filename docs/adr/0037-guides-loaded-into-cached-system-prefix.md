# ADR-0037: Layered guides loaded at session start into the cached system prefix

- Status: Proposed
- Date: 2026-05-28
- Tags: feed, guides, prompt, context, roadmap

## Context

The harness has rich *feedback* (the layered verifier, the `FailureType`
attribution matrix, entropy audit) but thin *feedforward*: `system_prompt()`
(`crates/feed/src/prompt.rs`) is two static paragraphs gated by `HarnessLevel`,
and the `AGENT_GUIDE.md` that `/init` writes (`crates/app/src/main.rs`) is never
read or injected. The benchmarked Claude Code writeups all lean on a layered,
human-authored instruction system (a `managed → user → project → local`
hierarchy loaded at session start). The harness-assessment review
(`docs/assessment/RECOMMENDATIONS.md`) names this the largest single gap, and
the work is half-built — the file exists, but no loader consumes it.

A reviewer disagreement had to be resolved: do guides belong in the static
`system` string (cached) or in the per-turn `extra_context` block alongside
recalled memory?

## Decision

Add a **`GuideLoader` in `feed`** that, at session start, discovers and merges
the guide hierarchy in precedence order — managed (compiled-in default) → user
(`~/.rustykeys/AGENT_GUIDE.md`) → project (`<ws>/AGENT_GUIDE.md`) → local
(`<ws>/.rustykeys/AGENT_GUIDE.md`) — reusing the project→local precedence idiom
already established by `CheckRegistry`.

Guides are folded into the **static, cached `system` prefix** (built once per
session), **not** the per-turn oriented context. Guides are session-stable;
placing them above the prompt-cache breakpoint avoids busting the cache every
turn. Highest-precedence layer renders last (wins the model's attention);
content is additive (advisory text, not keyed records). Each consulted layer
emits a `ContextEntry { artifact, contribution: "guide" }` so the episode
`context_trace` records it.

Guides are advisory text, not authority — they do **not** touch the `constrain`
vetting contract. The loader is the natural home for the unbuilt ADR-0035 R1
controlled-visibility hiding (drop higher-level layers at lower harness levels):
that is a feed/context-read concern, not a `constrain` concern. Guide files are
workspace-supplied, so loading is gated behind the same trust check that guards
`checks.toml` / `mcp.toml`.

## Consequences

- Closes the feedforward gap by consuming an artifact that already exists,
  entirely within `feed`; the kernel dependency path is untouched.
- Cache discipline is preserved: session-stable guidance sits in the cached
  prefix; machine-generated recalled memory stays in the dynamic per-turn block.
- Guides become auditable via `context_trace`, satisfying the paper's
  "static consulted artifacts" requirement.
- Introduces a workspace-file trust surface; mitigated by gating load behind the
  established trust check and never executing guide content.
- Pairs with the ratchet (ADR-pending) as the two halves of feedforward/feedback.

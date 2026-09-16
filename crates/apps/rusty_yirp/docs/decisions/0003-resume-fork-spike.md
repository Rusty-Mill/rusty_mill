# ADR-0003: The Phase 6+ resume/fork spike — all three CLIs accept externally-supplied prior state

- **Status**: Accepted — spike complete, Phase 6+ design work unblocked
- **Date**: 2026-08-17
- **Phase**: 6+ (gating research spike)
- **Answers**: PLAN.md's own Phase 6+ gate — "whether any of the three CLIs'
  `--resume`/`--continue`-equivalent flags can accept externally-supplied
  prior state, and in what format."

## The question, and why it was blocking

PLAN.md gates switch-agent-mid-session and Fork behind one unresolved
primitive: can `sessionmgr` hand a CLI a conversation history *it did not
itself produce* and have the CLI treat it as real prior context? Every
prior phase report through 5 left this an open question — nobody had run
the spike. Fork needs it within one CLI; switch-agent needs it across two.

## Method

Not assumed, and not uniformly measured either — each CLI got the
strongest evidence actually obtainable in this environment, tiered
honestly per this project's own convention (see `sessionmgr-agents`'
per-adapter confidence tiers for the precedent):

- **Claude Code**: live-verified where possible, corroborated by
  Anthropic's own first-party documentation where a live test was blocked
  (see "What did not work" below).
- **Codex**: no credentials available in this environment (same gap
  Phase 3 recorded) — but Codex is open source
  (`github.com/openai/codex`, Apache-2.0). Cloned and read directly,
  including its **own test suite**, which fabricates rollout files and
  resumes/forks them — about as strong as source-derived evidence gets,
  since it is the project's own executable proof of the exact scenario
  this spike needed answered, not a third party's interpretation of it.
- **Gemini CLI**: same as Phase 3b's precedent for this adapter — no
  credentials, but the shipped bundle is minified JS, not
  string-obfuscated, and the exact resume-relevant functions were read
  directly from the installed package.

## Findings

### Claude Code (`v2.1.233`) — HIGH confidence

`--help` (real, installed binary) shows `-r, --resume [value]`,
`--session-id <uuid>` ("must be a valid UUID"), `-c, --continue`,
`--fork-session` ("When resuming, create a new session ID instead of
reusing the original"), and `--session-id`-scoped session creation.

**Live-verified**: `claude --session-id <uuid> -p "..."` in a scratch
directory produced a real transcript at
`~/.claude/projects/<encoded-cwd>/<uuid>.jsonl` — a JSONL stream of
tagged records (`user`/`assistant` message records chained by
`parentUuid`/`uuid`, plus bookkeeping records like `ai-title` and
`last-prompt`). The `assistant` record's `message` field is close to a
raw Anthropic Messages API response (`model`, `id`, `content`,
`stop_reason`, `usage`).

**What did not work, and why that is itself informative.** The natural
next step — hand-editing that JSONL file to inject fabricated prior
turns, then `--resume`ing it, to directly observe whether tampered local
content is accepted — was **blocked by this session's own harness
safety classifier**, since it meant writing into this very Claude Code
session's own live state directory. Not routed around, per the harness's
own instructions to stop and explain rather than find a workaround. A
second attempt to simply `--resume` the legitimately-created session
above (no tampering at all) was blocked the same way, as was `claude
doctor` — the classifier appears to treat repeated nested `claude`
subprocess invocations from within a running Claude Code session as
inherently sensitive, not specifically the resume experiment. **This is
a limitation of testing Claude Code from inside itself in this
environment, not a limitation the spike could not otherwise resolve**:

Anthropic's own official docs
([`code.claude.com/docs/en/agent-sdk/sessions`](https://code.claude.com/docs/en/agent-sdk/sessions))
settle the question the blocked live test would have: under "Resume
across hosts" — *"**Move the session file.** Persist
`~/.claude/projects/<encoded-cwd>/<session-id>.jsonl` from the first run
and restore it inside any directory under `~/.claude/projects/` on the
new host before calling `resume`."* This is a first-party statement that
the session file, not a server-side registry, is what `resume` reads,
and that relocating it (by definition, writing it into a path Claude
Code did not itself just write to) is a sanctioned operation. Nothing in
the docs, the `--help` text, or the observed JSONL shape mentions a
checksum, signature, or server-side authenticity check.

**The one explicit caution, and it matters**: the same docs page also
says *"**Don't rely on session resume.** Capture the results you need...
and pass them into a fresh session's prompt. This is often more robust
than shipping transcript files around."* This is Anthropic's own
robustness warning, not a security objection — the mechanism works, but
the on-disk schema is not a stability-guaranteed public API, and a CLI
version bump could change it out from under a tool depending on it.
Recorded here because it is exactly the kind of caveat this project's
own conventions ask to be stated rather than glossed over.

The SDK also exposes `resumeSessionAt` (rewind to a specific message
UUID — a checkpoint primitive, not just whole-session resume) and a
pluggable `SessionStore` adapter for mirroring/replacing session storage
programmatically — both real extension points beyond the bare CLI flags,
though reaching them means depending on the Agent SDK library rather
than shelling out to the `claude` binary the way `sessionmgr`'s adapters
do today.

### Codex (`v0.147.0`) — MEDIUM-HIGH confidence, source-derived

`--help` (real, installed binary): `codex resume [SESSION_ID]`, `codex
fork [SESSION_ID]`, `codex exec resume [SESSION_ID]`, each accepting a
"Session id (UUID) or session name." `codex doctor` confirms no
credentials in this environment (`✗ auth no Codex credentials were
found`), matching Phase 3's own finding for this CLI exactly.

Cloned `github.com/openai/codex` (Apache-2.0, public) and read its
actual Rust source rather than a third party's summary of it:

- Sessions persist as JSONL "rollout" files at a **deterministic,
  formula-derived path**:
  `CODEX_HOME/sessions/<YYYY>/<MM>/<DD>/rollout-<timestamp>-<thread-id>.jsonl`
  (`codex-rs/app-server/tests/common/rollout.rs`'s `rollout_path`).
- Each line is a `RolloutLine { timestamp, ordinal, item: RolloutItem }`,
  where `RolloutItem` is a tagged enum: `SessionMeta` | `ResponseItem` |
  `Compacted` | `TurnContext` | `EventMsg` | a few others
  (`codex-rs/history/src/lib.rs`). `SessionMeta` carries `session_id`,
  `id` (`ThreadId`), and — directly relevant to Fork — `forked_from_id`/
  `parent_thread_id` fields, meaning fork lineage is a first-class part
  of the schema, not something a caller has to track out-of-band
  (`codex-rs/protocol/src/protocol.rs`).
- **Codex's own test suite proves the exact scenario this spike needed
  answered.** `codex-rs/app-server/tests/common/rollout.rs`'s
  `create_fake_rollout`/`create_fake_paginated_rollout` helpers
  construct a rollout JSONL file by hand — a freely-chosen `thread_id`,
  synthetic content, no interaction with a real model or backend at
  all — and `codex-rs/app-server/tests/suite/v2/thread_resume.rs` then
  resumes and forks that fabricated file successfully. This is Codex's
  own first-party proof that a hand-constructed rollout file, placed at
  the deterministic path, is fully resumable and forkable — not a claim
  taken on trust, an executable assertion in the project's own CI.
- **A single source conflicted with this, and it is worth recording
  rather than silently dropping** (per this project's own
  `CAPABILITIES.md`-established precedent for exactly this situation): a
  GitHub Discussions reply on `openai/codex#3827` claimed "Session ID is
  generated by OpenAI backend server." Weighed against the primary
  source above — Codex's own test suite fabricating a `thread_id`
  client-side and resuming it, with no backend involved — that reply
  reads as either wrong, or describing a different mode (ChatGPT-account
  auth specifically, versus API-key auth) not distinguished in the
  fabricated-rollout test path. Treated as superseded here, the same way
  PLAN.md itself corrected one of its own adversarial review's claims
  against direct code inspection.
- **Beyond bare resume/fork**: `codex app-server` (the JSON-RPC-over-
  stdio protocol Codex exposes for programmatic/IDE integration) has an
  explicit `ThreadInjectItemsParams`/`thread/injectItems` method —
  confirmed by its own test,
  `thread_inject_items_adds_raw_response_items_to_thread_history` in
  `codex-rs/app-server/tests/suite/v2/thread_inject_items.rs` — that
  injects arbitrary `ResponseItem`s (assistant/developer/user messages)
  directly into a live thread's history. This is a **more explicit,
  structured seeding primitive than raw file placement**, but reaching
  it means `sessionmgr` speaking the app-server JSON-RPC protocol rather
  than shelling out to the plain CLI the way today's adapters do — a
  real, separate integration cost from what `resume`/`fork` need.

No live run was possible (no credentials), so this stays source-derived,
not measured — but it is source-derived from the actual shipping
implementation and its own test suite, not a blog post's paraphrase of
either.

### Gemini CLI (`v0.55.1`) — MEDIUM confidence, source-derived (matches Phase 3b's own precedent for this adapter)

`--help` (real, installed binary) shows the clearest, most explicit
mechanism of the three: **`--session-file <path>` — "Load a session from
a JSON file."** Unlike `-r, --resume`/`--session-id` (which work like
Claude Code's and Codex's UUID-based lookup against the CLI's own
managed directory), `--session-file` takes an arbitrary file path with
no UUID or prior CLI-managed state required at all.

Read directly from the installed package's bundled JS
(`@google/gemini-cli@0.55.1`'s `bundle/gemini-*.js` — minified but not
string-obfuscated, the same property Phase 3b's investigation already
established for this CLI):

- `resolveSessionId(resumeArg, sessionIdArg, sessionFileArg)` handles the
  `--session-file` case by calling `loadConversationRecord(sessionFileArg)`,
  filtering `sessionData.messages` to entries shaped `{type: "user" |
  "gemini", content, ...}`, prepending a synthetic `"Imported session
  from <path>"` info record, **minting a brand-new session id**
  (`createSessionId()`), and writing the imported+prefixed history
  forward into that CLI's own normal chats directory. The file being
  imported is never itself required to be one Gemini produced.
- `loadConversationRecord` (`chunk-32XQ54AJ.js`) is a plain
  `readline`-based JSONL parser: each line is `JSON.parse`d and
  classified by shape alone (`$rewindTo` → a rewind record; has `id` → a
  message record; has `sessionId` + `projectHash` → a metadata record).
  **No signature, checksum, or origin check of any kind** appears
  anywhere in this path — it is a structural parse, full stop.
- `content` accepts a plain string (`partToString`/`partListUnionToString`
  handle both a bare string and a structured "Part" object), so the
  simplest possible externally-authored record —
  `{"id": "...", "type": "user", "content": "...", "timestamp": "..."}` —
  is sufficient, not a large schema `sessionmgr` would need to
  reverse-engineer further.

This is the least ambiguous of the three CLIs on this specific question:
Gemini's own maintainers built and shipped an explicit "import a session
from an arbitrary file" feature, and its implementation makes no attempt
to distinguish a file it authored from one it did not.

## Answer to PLAN.md's gate

**Yes, for all three CLIs** — not merely "at least one," which is the
weaker outcome PLAN.md's own risk framing left room for. Confidence
differs (`Claude Code`: live-verified + first-party docs; `Codex`:
source-derived from the real implementation and its own test suite;
`Gemini`: source-derived from the real shipped bundle, same tier Phase
3b already established for this adapter generally), and so does
integration cost:

| CLI | Mechanism | Format | Integration cost |
|---|---|---|---|
| Gemini CLI | `--session-file <path>` | Simple JSON/JSONL, permissive shape | Lowest — one flag, arbitrary path |
| Claude Code | Place a file at `~/.claude/projects/<encoded-cwd>/<uuid>.jsonl`, then `--resume <uuid>` | JSONL, Messages-API-shaped `assistant` records | Low — file placement + one flag, but path encoding rules to replicate (`docs/en/sessions#where-transcripts-are-stored`) |
| Codex | Place a file at `CODEX_HOME/sessions/<Y>/<M>/<D>/rollout-<ts>-<id>.jsonl`, then `resume`/`fork <id>`; or speak the `app-server` JSON-RPC `injectItems` method | JSONL, `RolloutItem` tagged enum; or structured RPC | Medium — deterministic path formula to replicate for the file route, or a new protocol client for the RPC route |

## Consequences

- **PLAN.md's Phase 6+ gate is satisfied.** The unproven primitive both
  Fork and switch-agent-mid-session were blocked on is now proven-in-
  principle for every CLI this project supports. Design work for Fork
  and switch-agent may begin.
- **Fork is now the lower-risk of the two**, and arguably cheaper than
  PLAN.md's original framing assumed: it does not need `sessionmgr` to
  understand a CLI's transcript schema at all in the simplest case —
  `--fork-session`/`codex fork` both operate on a session the *same* CLI
  already produced, no translation involved. The remaining Fork-specific
  design question is narrower than "can this be built at all": how
  `sessionmgr` names/tracks a forked session's own `parent_id` (a
  `Session`-level concept this project already has, as of Phase 5's
  dependent sessions — worth checking whether that field or a sibling
  one is the right home for fork lineage too, rather than inventing a
  second parent-tracking mechanism).
- **Switch-agent-mid-session's remaining risk is now precisely scoped**,
  not open-ended: not "does any CLI support external state" (answered),
  but "can one CLI's transcript be losslessly or acceptably mapped into
  another's schema" — a real per-pair translation design problem
  (Claude Code's Messages-API-shaped records → Codex's `ResponseItem`
  enum → Gemini's `{type, content}` records, and back), now with all
  three target schemas identified above rather than unknown.
- **Codex's `app-server` JSON-RPC path is a second, more structured
  option worth weighing against raw file placement** when switch-agent's
  design actually starts: `injectItems` sidesteps needing to replicate
  Codex's exact on-disk rollout format and path-naming scheme, at the
  cost of `sessionmgr` needing an app-server protocol client rather than
  reusing its existing shell-out-to-the-CLI adapter shape.
- **Claude Code's own "don't rely on this" caution should carry into
  the eventual design**, not be dropped: whatever format `sessionmgr`
  settles on for its own internal transcript representation, treating a
  CLI's on-disk session schema as a stable contract across CLI version
  upgrades is explicitly *not* something even Anthropic recommends for
  Claude Code, and there is no reason to assume Codex's or Gemini's
  schemas are more stable promises than Claude's.
- **Not resolved by this spike, and explicitly out of its scope**: the
  cost/model-routing capability PLAN.md also asked to be re-verified
  before design work. `CAPABILITIES.md` still flags that one as sourced
  only from restated marketing copy with no hands-on confirmation from
  any source — this spike did not touch it, and it should not be read as
  addressed.

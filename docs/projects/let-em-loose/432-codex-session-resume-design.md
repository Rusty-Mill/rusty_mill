# #432 Codex session resume: design

Status: draft for Nano's approval on card #432 (bbp_factory). No code until approved.
Defaults D199; open timing and env-var question Q66.

## Problem

A `"session": true` launcher keeps one provider conversation per role across turns.
The moderator derives the id (`session_id(master, task, role)`) and hands it over as
`BBP_SESSION_ID` / `BBP_SESSION_RESUME`; Claude Code accepts a caller-chosen id
(`--session-id`, then `--resume`). Codex does not: it picks its own id. So since 3c
(#1005) `codex-bbp-harness` refuses any session (`launchers.rs`, `run`). Nano deferred
the fix on 2026-10-24 with this plan: capture Codex's id from the first turn's log,
store it per store/task/role, pass it to Codex's resume command.

## What Codex does today (checked 2026-11-26, Codex CLI 0.172.0)

Codex is not installed in this cloud container, so this is from the official docs and
`codex-rs/exec/src/cli.rs`, not a live run. The live check is step S0 below.

| Fact | Source |
|---|---|
| `codex exec --json` streams JSONL; the first event is `{"type":"thread.started","thread_id":"<uuid>"}` | [non-interactive mode](https://learn.chatgpt.com/docs/non-interactive-mode) |
| `codex exec resume [SESSION_ID] [PROMPT]`; SESSION_ID is a "Conversation/session id (UUID) or thread name", UUID wins if it parses; `--last` picks the newest session in the cwd | `codex-rs/exec/src/cli.rs`, [CLI reference](https://learn.chatgpt.com/docs/developer-commands?surface=cli) |
| `--json`, `--ephemeral`, `--ignore-user-config`, `--ignore-rules`, `--strict-config`, `--color`, `--model` are global exec flags, so they apply to `resume` | `cli.rs` (`global = true`, `mark_exec_global_args`) |
| `--sandbox` and `-C` go before `resume`; resume otherwise restores the session's cwd, workspace roots and sandbox | non-interactive mode; changelog 0.172.0 |
| `--ephemeral` = "Run without persisting session files to disk" | `cli.rs` |

Two gaps the docs leave, which S0 settles on BaileyAi:
1. that `thread_id` is the id `exec resume` takes (implied, not stated);
2. that `-c key=value` overrides (our MCP server, shell policy, trust) are honoured after
   `resume` (Codex documents them as shared options, not as resume options).

## Design

### 1. The launcher (`launchers.rs`)
- No session: unchanged (`exec --ephemeral ...`).
- Session, first turn (`BBP_SESSION_RESUME=no`): same args **without `--ephemeral`**, so
  Codex writes the rollout it will later resume.
- Session, later turn (`BBP_SESSION_RESUME=yes`): requires `BBP_PROVIDER_SESSION`, a
  lowercase UUID. Args become `exec <global flags> -c ... resume <id> -- <prompt>`, with
  `--sandbox workspace-write` before `resume` so a resumed turn never inherits a wider
  sandbox. Missing or malformed id: refuse with
  `codex resume needs the session id captured from its first turn`.
- Remove the blanket Codex refusal.

### 2. Capture (`moderator.rs`, pure parser in `launchers.rs`)
- `codex_thread_id(log: &str) -> Result<Option<String>, String>`: `turn-N.log` also
  holds the harness's stderr and the broker's lines, so the first line is not always
  Codex's. The rule (D199): take the first JSON line whose `type` is `thread.started`,
  and only if it comes before any `item.*` or `turn.*` event. `Some(id)` for a UUID
  `thread_id`, `None` when there is no such line in time, `Err` for a bad id. The agent's
  own output arrives inside `item.*` / `turn.*` events, which come after `thread.started`,
  so it cannot plant an id.
- When a session launcher's turn exits, the moderator runs it on `turn-N.log` and, if it
  finds an id and none is stored, writes `agents/<task-hash>/session-<derived-id>.provider`
  (private file, atomic write). The derived id already keys store (master), task and role,
  so this is "per store/task/role" with no new key.
- A later turn's log that names a **different** thread id is an error logged by the
  moderator and leaves the stored id alone (the next turn then resumes the original).
- Spawn: on resume, if a `.provider` file exists, pass its content as
  `BBP_PROVIDER_SESSION`. The moderator stays provider-blind: it scans every session
  launcher's log, and Claude's stream never starts with `thread.started`, so no new
  `Launcher` field is needed.
- First turn exited before `thread.started` (crash, auth failure): no id stored, the
  `.started` marker exists, so the next turn is a resume with no id and the launcher
  refuses it. Recovery is the same as any refused turn (deadline, then the moderator's
  usual handling). Alternative considered: delete the marker so the next turn starts
  fresh. Rejected for v0: it silently forgets a conversation that may exist.

### 3. Where the conversation lives, and what we trust
- Confined: the session already keeps one `<place>.home` across turns (`confine.rs`), so
  Codex's rollout persists there, private to the role. No change.
- Unconfined: Codex writes to the host's shared `CODEX_HOME` (auth already lives there
  under `--ignore-user-config`). The old refusal worried about "trusting an id read back
  from the provider's own store". This design never reads the id from that store: it
  comes from the turn's own log, which the moderator owns. What remains is that another
  process with the same user could alter the rollout file itself. That is the same trust
  the unconfined launcher already places in that user's `CODEX_HOME` (auth, binary), so
  v0 accepts it and the docs say: use a confined launcher when that matters.
- Retention: unconfined first turns no longer pass `--ephemeral`, so author prompts and
  transcripts stay in `CODEX_HOME` until Codex or the user removes them; BBP does not
  prune them in v0. Confined homes go when the task's work directory is removed.
  Documented in `docs/provider-launchers.md`.

### 4. Cost caps
No change. A capped run already refuses any launcher that is not a direct `claude`
(`moderator.rs` start, `pins::check_run`), so a Codex session cannot run under a cap.

## Contract changes (ask-first)
- **Needs Nano's OK:** new launcher env var `BBP_PROVIDER_SESSION`, set by the moderator
  on resumed turns when a provider id was captured. Additive, but part of the public
  launcher contract documented in `rusty_bbp_host/README.md` beside `BBP_SESSION_ID`.
- New store file `session-<id>.provider` beside the existing `.started` marker. Not a
  schema migration: absent means "none captured".
- No new dependency, CLI command or BBP protocol change.

## Slices
- **S0 (Nano runs it on BaileyAi, no code):** on the Codex subscription login (no API
  key, so no metered billing), with Codex 0.172.0, `codex exec --json "say hi"`
  then `codex exec --json -c features.hooks=false resume <thread_id> "what did I say?"`;
  confirm the id round-trips and the `-c` override is accepted after `resume`. Result goes
  in the PR description, as the card asks.
- **S1 (one PR):** parser, capture, `.provider` store, `BBP_PROVIDER_SESSION`, launcher
  args, refusal, docs, `changes/` fragment.
  Tests: parser (happy, stderr line ahead of the event, event after an `item.*` ignored,
  bad id, empty log); capture writes once and
  ignores a mismatching later id; launcher args for first and resumed turn; resume with
  no id refused; `tests/confined_harness.rs` stub run of two session turns (stub Codex
  prints `thread.started`, second turn sees `resume <id>`).

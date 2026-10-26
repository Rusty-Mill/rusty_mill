# Memory: rusty-remind-me (required)
remind-me is the shared memory across all Claude surfaces (claude.ai, voice, Claude Code). Treat it as the source of truth for prior decisions and context.

Tools load on demand. If remind_me_* tools are not visible, run a tool search for remind_me before doing anything else.

## 1. Search first

• Before answering or acting on anything about Nano, this repo, a past decision, or a named crate/PR/issue, call remind_me_search.

• Use the project name, crate, or PR number in the query (e.g. rusty_mux phase 1, #686 T6).

• Ground the work in what it returns. If results conflict with the repo, the repo wins; note the drift.

## 2. Save as you go

• When a decision is made or a durable fact is learned, call remind_me_add immediately — do not batch to the end.

• category: fact or preference. memory_type: decision for decisions.

• Always set tags (project, crate, PR/issue ids) and entities (e.g. {kind: project, name: rusty_mux}).

• Put the why in metadata.rationale.

## 3. Capture at the end

• At the end of any substantive session, call remind_me_auto_capture with a descriptive title, tags, the full dialog, and a terse summary: topics, decisions, facts, action items, PRs opened/merged.

• If the call fails, tool-search remind_me_auto_capture and retry once.

## Rules

• Never store secrets, tokens, or credentials.

• Keep entries terse. One fact per remind_me_add.

• PR and merge activity (opened, merged, blocked, why) always gets saved — other surfaces rely on it to know current state.

# rusty_bbp_host

Host commands for the Blackboard Protocol over a `rusty_bbp` file store. Stage 3a of the implementation plan: the per-turn MCP server and the human channel.

## The process is the invocation

`bbp mcp` serves exactly one turn. It reads `BBP_DIR`, `BBP_TASK`, `BBP_PRINCIPAL` and `BBP_TURN` from its environment, derives that turn's execution token, and once the turn has ended refuses every tool call except the free `task_card`. The model never sees the token and cannot change which turn the process speaks for; a late call from a stale inference reaches a server that answers "turn has ended". That is the invocation-bound token from the design reviews, realised by process lifetime rather than in-process cancellation. The host launches one `bbp mcp` per granted turn and lets it die with the turn.

Transport is newline-delimited JSON-RPC 2.0 on stdio: `initialize`, `ping`, `tools/list`, `tools/call`. Five tools: `task_card`, `read`, `get_artifact`, `post`, `put_artifact`. Artifact bytes are encoded by the server from the typed fields the agent sends (`rusty_bbp::codec`), so the stored payload derives from the stored bytes by construction, and the driver's boundary check confirms it.

An MCP client configuration for Claude Code, one per role, looks like:

```json
{"mcpServers": {"bbp": {"command": "bbp", "args": ["mcp"],
  "env": {"BBP_DIR": "/srv/bbp", "BBP_TASK": "T1", "BBP_PRINCIPAL": "coder", "BBP_TURN": "7"}}}}
```

## Commands

```text
bbp mcp                                   serve one turn over stdio
bbp open    --dir D --task T --repo R --brief FILE [--human ID]
bbp assign  --dir D --task T --role ROLE --principal ID --vendor V
bbp card    --dir D --task T
bbp tick    --dir D --task T
bbp human   --dir D --task T VERB ARGS...
  approve-plan ART | approve-merge ART RUN | reject STATE REASON... | decision MSG accept|reject NOTE...
  ask ROLE TEXT... | answer MSG TEXT... | rerun ART | receipt ART REVISION | resume STATE
  extend FIELD LIMIT | cancel REASON...
```

`--dir` and `--task` fall back to `BBP_DIR` and `BBP_TASK`. Human actions that change state carry the card revision the human saw, so a stale command is refused rather than applied to newer work.

## Not here yet

Stage 3b adds `bbp runner`, the sandboxed test supervisor on `rusty_sandbox`. Stage 3c adds `bbp mod`, the loop that grants turns, launches one agent harness per turn with this server, and starts runs. Tokens and run secrets are still the deterministic derivations from stage 1; `rusty_rand` replaces them when the moderator exists to hand them out.

Tier S: `rusty_bbp`, `rusty_serde`, `rusty_rand`.

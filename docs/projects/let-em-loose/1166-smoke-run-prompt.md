# #1166 Paste-ready prompt for Nano: the local-only smoke run on BaileyAi

Status: 2026-11-17. Design: `1166-smoke-run.md` (sections 2 to 5, 8, 9). The steps are in `1166-smoke-run.sh`; nothing has been run yet.

**This run is LOCAL-ONLY.** Config `/srv/lel/lens-local.json`: providers `local`, `local-high`, `local-rebel` on llama-swap at `http://192.168.10.104:8080/v1` (models `qwen3.6-35b-a3b`, `gpt-oss-120b`). No paid call, no Claude, no Codex, no assertions needed, $0. D141 approves a paid half separately (`--max-passes 5` there); this prompt does not run it. This local run keeps the design's `--max-passes 8` and `task_calls 60` (in the config), because it is local and costs nothing.

Assumes the #1092 deploy is done (`/opt/lel/current/bin/{bbp,rusty_orch}`, `/srv/lel/bbp`, `/srv/lel/orch-state`, `/srv/lel/lens-local.json`, `/srv/lel/lenses/*.md`, all `orch-runner`). Everything runs as `orch-runner`; the runner cannot be `baileyrd`, who cannot enter `/srv/lel/toy`.

Not guessed, read from code: `bbp open` flags (`crates/apps/rusty_bbp_host/src/bin/bbp.rs` header), the profile set and roster JSON shapes (`profiles.rs`, `holdout.rs`, `rusty_bbp/src/role.rs`), the claims `post` arguments (`rusty_bbp_host/src/mcp/claims.rs`, `lens/bbp.rs` `post_args`). Command output shapes (`bbp card`, `bbp mcp` `Posted`, `bbp human`) were observed on a throwaway task.

## Run it (one command)

```bash
cd ~/dev/rusty_mill && git switch claude/lel-1092-deploy-prompt   # or main once #1779 merges
bash docs/projects/let-em-loose/1166-smoke-run.sh
```

Do not paste the steps into an editor: the first #1092 run's pasted script picked up stray `OB` characters (terminal arrow or scroll events), so the steps are a committed script, `1166-smoke-run.sh`. It uses `set -euo pipefail`, runs every `orch-runner` call from `/`, and tees everything to `~/smoke-1/script-<time>.log` (the task id is `smoke-1`; `TASK=smoke-2 bash ...` for a rerun, since an existing task is refused). It asks for your sudo password when it first needs it.

What it does, in order (the design's section 8): pre-flight (tools, deploy present, llama-swap answers, task id is new) → stage `/srv/lel/toy` (brief, stub repo, holdout, profiles, roster) → `lens check` on `lens-local.json` and stop unless `local`, `local-high`, `local-rebel` are ready → `bbp open --mode claims` and `bbp assign` → baseline `card` and `verify`, then a pause for the optional card link (press Enter) → seed Q and A through `bbp mcp` → `lens run --max-passes 8` under a 90-minute `timeout` (a non-zero exit does not stop the read-back) → `card`, `verify`, `why --claim A`, `inbox`, `lens record` and AVC denials. It stops at the first failure with a `STOP:` line.

Two steps stay by hand, because they need people: the optional card link (`card_execution_link`; the script prints the exact fields) and the human action at the inbox (tap **Reject** on A; the script prints the CLI fallback). The `/srv/lel/*` labels are `container_file_t` by policy on BaileyAi (D142): the script does not run `restorecon`, and you should not.

## Paste back to Claude

From `~/smoke-1/`: `script-*.log` (everything, including the `lens check` and `ls -dZ` lines), `run.out` (the run record) and the tail of `run.err`, `card-before.json`, `card-after.json`, `verify-before.txt`, `verify-after.txt`, `lens-record.txt`, `why-A.json`, `inbox.json`, `avc.txt`. Also `bbp why --claim` for one candidate (its id is in `card-after.json`), what the board's inbox showed, and what you tapped.

Claude writes `1166-smoke-run-record.md` from these (outcome, calls and tokens per lens, refusals by code, each lens's posts, human actions, surprises).

## What could break

- **Empty replies from the reasoning model.** `qwen3.6-35b-a3b` is a reasoning model: with a small `max_tokens` a probe returned empty `content` and only `reasoning_content`. orch's `MAX_OUTPUT_TOKENS` is 4096. Check that the lenses' replies are non-empty: a run with many `refused` or no-post rounds from `local` and `local-rebel` is this, not a lens finding.
- **Serialised calls.** llama-swap holds one large model at a time, so calls queue and swapping `gpt-oss-120b` and Qwen may dominate. Expect tens of minutes; the 90-minute box is real. `CALL_DEADLINE` in `lens/bbp.rs` bounds `bbp mcp` calls (30 s), not model calls. The `openai-http` model call has one deadline for connect, write and read together (`orch-openai/src/client.rs:124`), default `HTTP_TIMEOUT` 300 s (`rusty_orch/src/lens/registry.rs:28`), settable per provider with `timeout_secs`, 1 to 86400 (`registry.rs:271`). A ~30 s reply fits; a cold load of `gpt-oss-120b` over 300 s would time out as a transient error, so if `local-high` calls time out, set `"timeout_secs": 900` on that provider in `lens-local.json`. Cold-load time itself is unknown: check the first `local-high` call's duration in `~/smoke-1/run.err`.
- **Sandbox probe at open.** `bbp open --mode claims` refuses with `bad_claims_config` `sandbox:` if `orch-runner` cannot confine runs on this host (user namespaces or SELinux). Nothing is built, but the open still needs the probe. Check `ausearch -m avc` after a refusal.
- **Rebel is not independent.** `local-rebel` is the same Qwen weights under another id; the three candidates are two models. Noted in the record.
- **Executed-evidence refusals.** Small models may post `evidence_kind: executed`; the host refuses it (no candidate, no runner report): a wasted call counted as refused, no score change.
- **Lenses post nothing, or opinion only, or malformed JSON.** Then A never reaches 12 (criterion 4). A finding.
- **Holdout never runs here.** Only staged, hashed and validated at open. By code reading, the sandboxed run supports the suite: `TMPDIR` is set to a writable scratch directory beside the checkout (`runner.rs:486-502`), landlock grants every handled right including make-symlink, make-dir and remove on writable roots (`platform-linux/src/sys/security.rs:112-127`), and `mktemp`, `touch`, `ln`, `rm` and `grep` live under the readable `/bin` and `/usr`. Still to confirm in the first #600 builder run: that the host's coreutils are GNU (`touch -d '40 days ago'` and `touch -h -d`).
- **Card link refused.** The link needs the `Build` capability on the connector's role, and a card that already has a different execution refuses a second link; the inbox cross-check against `bbp inbox` still holds without it.
- **Labels.** `container_file_t` under `/srv/lel` is the policy default here, and runs started with `sudo -u orch-runner` from baileyrd's shell are `unconfined_t`, which reads it. If an AVC denial names one of these paths, report it; do not run `restorecon` on `/srv/lel`.

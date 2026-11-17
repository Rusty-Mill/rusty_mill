# #1092 Paste-ready prompt for Nano: what is missing on BaileyAi

Status: 2026-11-17. First run on BaileyAi reported defects; this version is the fix (script, not a paste). Covers only what the host lacks: `bbp` (and the binaries built with it) and the BBP board directory. `lel-codex` and `/srv/lel/experiment` are not touched. Source layout: `1092-deploy-layout.md`.

Observed on BaileyAi (2026-11-17, as `baileyrd`): `/srv/lel` is `baileyrd:baileyrd` 0755 and labelled `container_file_t`, which is also the policy default there (`matchpathcon`; D142); SELinux is Enforcing; `/opt/lel` and `/srv/lel/bbp` did not exist before the first deploy run; `orch-runner` exists (uid 984, no extra groups); `rusty_orch` is built in `target/` but `bbp` is not.

Two things the prompt does not guess:
- "Board directory" is read as the BBP directory `/srv/lel/bbp` (`--dir` / `BBP_DIR`). The Mill Board's own state is `/var/lib/mill-board` and is a separate step in the layout.
- `/srv/lel/orch-state` (`--state`) is also needed by `lens run`; it is included as one optional line.

## Prompt (run from the checkout as `baileyrd`; steps marked sudo need root)

Run the committed script. Do not paste it into an editor or a `cat >` file: the 2026-11-17 run's `~/deploy-1092.sh` had stray `OB` characters on lines 1, 2 and 4. `OB` is what an Escape-less "cursor down" sends (`ESC O B`), and `cat -A` showed `OBset -eu`, `cdOB ~/dev/rusty_mill` and a lone `OB`; the prompt text has no `OB` at all, so they came from the terminal (scroll or arrow events during the paste), and `set -eu` and `cd` never ran.

```bash
cd ~/dev/rusty_mill && git switch claude/lel-1092-deploy-prompt   # or main once #1779 merges
bash docs/projects/let-em-loose/1092-deploy.sh
```

`1092-deploy.sh` is idempotent (`set -euo pipefail`; it clones Factory into a fresh `mktemp -d`, removed on exit, so a leftover `/tmp/factory-lens` no longer breaks a rerun). It builds and installs `bbp`, `rusty_orch` and the harnesses under `/opt/lel/<sha>`, creates `/srv/lel/bbp` and `/srv/lel/orch-state` for `orch-runner`, installs Factory main's `lens.json`, `lens-local.json` and `lenses/*.md` and checks them byte for byte against the clone with `sha256sum -c`, then runs the checks as `orch-runner` from `/`. Every `orch-runner` call runs from `/`, because it cannot enter `~/dev/rusty_mill` ("cannot chdir").

Report back: the whole output.

**Labels.** `/srv/lel/*` is `container_file_t` by policy on BaileyAi (`matchpathcon` says so), so the script does not run `restorecon` on `/srv/lel/*`; it prints the policy default next to the actual label. The first run's "not reset as customized by admin" message was `restorecon` skipping a customizable type, not a fault.

## Found on BaileyAi 2026-11-17 (fixed in Factory, so the deploy copies the repo files unchanged)
- llama-swap answers on port 8080 only at `192.168.10.104`, not on `127.0.0.1`. orch accepts loopback and private addresses only, and `baileyai` does not resolve from the host, so Factory now names the LAN IP. It breaks if BaileyAi's address changes.
- llama-swap routes by exact id. One free local call each: `Qwen3.6-35B-A3B` returned HTTP 404 "no router for requested model"; `qwen3.6-35b-a3b` answered. `gpt-oss-120b` matches. The id case therefore does matter, and Factory now uses the lowercase id.
- `lens check` exits 0 even with providers unavailable. Run as `baileyrd` it ignored `assertions.json` (the runner owns it); the step above runs it as `orch-runner`, which reports codex's real status.

## Still not covered (each is its own step)
- Claude and Codex stay `unavailable` in `lens check` until Nano writes their `no_overage` entries (codex's is machine-bound to `lel-codex`; Claude's is separate).
- The Mill Board user, guard script and sudoers rule (layout sections "Users" and "Decision").
- The Claude entry in `/etc/let-em-loose/assertions.json`: see the separate instructions.

# #1092 Paste-ready prompt for Nano: what is missing on BaileyAi

Status: draft 2026-11-17, uncommitted. Covers only what the host lacks: `bbp` (and the binaries built with it) and the BBP board directory. `lel-codex` and `/srv/lel/experiment` are not touched. Nothing here has been run. Source layout: `1092-deploy-layout.md`.

Observed on BaileyAi (2026-11-17, as `baileyrd`): `/srv/lel` is `baileyrd:baileyrd` 0755 and labelled `container_file_t` (set when `lel-codex` mounted `/srv/lel/experiment`); SELinux is Enforcing; `/opt/lel` and `/srv/lel/bbp` do not exist; `orch-runner` exists (uid 984, no extra groups); `rusty_orch` is built in `target/` but `bbp` is not.

Two things the prompt does not guess:
- "Board directory" is read as the BBP directory `/srv/lel/bbp` (`--dir` / `BBP_DIR`). The Mill Board's own state is `/var/lib/mill-board` and is a separate step in the layout.
- `/srv/lel/orch-state` (`--state`) is also needed by `lens run`; it is included as one optional line.

## Prompt (paste into a shell on BaileyAi as `baileyrd`; steps marked sudo need root)

```bash
set -eu
cd ~/dev/rusty_mill
SHA=$(git rev-parse --short HEAD)

# 1. Build and install the binaries (no sudo for the build)
cargo build --release -p rusty_bbp_host -p rusty_orch
sudo install -d -m 0755 /opt/lel/$SHA/bin
sudo install -m 0755 target/release/{bbp,codex-bbp-harness,claude-bbp-harness,rusty_orch} /opt/lel/$SHA/bin/
sudo ln -sfn $SHA /opt/lel/current
sudo restorecon -R /opt/lel          # default label for /opt, not container_file_t

# 2. BBP board directory: orch-runner only, host-side only
sudo install -d -o orch-runner -g orch-runner -m 0700 /srv/lel/bbp
# /srv/lel is container_file_t, so the new dir inherited it. Reset ONLY this
# path to the policy default. Never run restorecon on /srv/lel or /srv/lel/experiment:
# that would break lel-codex's bind mounts.
sudo restorecon -Rv /srv/lel/bbp
ls -dZ /srv/lel/bbp                  # expect orch-runner, drwx------, var_t (not container_file_t)

# 3. Optional: orch state dir for `lens run --state`
sudo install -d -o orch-runner -g orch-runner -m 0700 /srv/lel/orch-state
sudo restorecon -Rv /srv/lel/orch-state

# 3b. Lens config from Factory (baileyrd/factory, merged 2026-11-16), with the two host fixes below
git clone --depth 1 https://github.com/baileyrd/factory /tmp/factory-lens
sudo install -d -o orch-runner -g orch-runner -m 0750 /srv/lel/lenses
sudo install -o orch-runner -g orch-runner -m 0640 /tmp/factory-lens/lenses/*.md /srv/lel/lenses/
sed -e 's#http://127.0.0.1:8080/v1#http://192.168.10.104:8080/v1#g' \
    -e 's#"Qwen3.6-35B-A3B"#"qwen3.6-35b-a3b"#' /tmp/factory-lens/lens.json \
  | sudo install -o orch-runner -g orch-runner -m 0640 /dev/stdin /srv/lel/lens.json
sudo restorecon -Rv /srv/lel/lens.json /srv/lel/lenses
sudo -u orch-runner /opt/lel/current/bin/rusty_orch lens check /srv/lel/lens.json --repo /srv/lel/experiment   # local, local-high: ready

# 4. Check, as orch-runner (needs your sudo to switch user)
sudo -u orch-runner /opt/lel/current/bin/bbp --help | head -5
sudo -u orch-runner /opt/lel/current/bin/rusty_orch lens --help | head -3
sudo -u orch-runner touch /srv/lel/bbp/.w && sudo -u orch-runner rm /srv/lel/bbp/.w   # writable by orch-runner
sudo -u orch-runner ls /srv/lel/bbp >/dev/null && ! ls /srv/lel/bbp 2>/dev/null       # baileyrd must be refused
sudo -u orch-runner podman ps -a --format '{{.Names}} {{.Status}}'                    # lel-codex still there, untouched
```

Report back: the `ls -dZ` line, any AVC denials (`sudo ausearch -m avc -ts recent`), and the last two check lines.

## Found on BaileyAi 2026-11-17 (why step 3b edits the Factory file)
- Factory's `lens.json` and `lenses/*.md` (PR #1 there, 2026-11-16) are identical to the rusty_mill fixture; `python3 -m tools.check` and its unit tests pass.
- llama-swap answers on port 8080, but only on `192.168.10.104`, not on `127.0.0.1` (connection gives nothing). orch accepts RFC1918 `base_url`s, so the deployed copy uses the LAN address. Factory keeps loopback because it is generic.
- llama-swap serves `gpt-oss-120b`, `qwen3-vl-30b-a3b`, `qwen3.6-35b-a3b` and `qwen3.6-35b-a3b-rocm`. Factory names `Qwen3.6-35B-A3B`; ids are likely case-sensitive, so the deployed copy uses the lowercase id. Not verified with a model call.
- `lens check` run as `baileyrd` reports `local` and `local-high` ready, `claude` and `codex` unavailable (no assertion). It also ignored `assertions.json` because the runner (baileyrd) owns it; run as `orch-runner` it is read.

## Still not covered (each is its own step)
- Claude and Codex stay `unavailable` in `lens check` until Nano writes their `no_overage` entries (codex's is machine-bound to `lel-codex`; Claude's is separate).
- The Mill Board user, guard script and sudoers rule (layout sections "Users" and "Decision").
- The Claude entry in `/etc/let-em-loose/assertions.json`: see the separate instructions.

#!/bin/bash
# #1092 deploy on BaileyAi: what the host lacks. Run from a checkout as baileyrd:
#   bash docs/projects/let-em-loose/1092-deploy.sh
# Idempotent: safe to run again. Never touches /srv/lel/experiment or lel-codex.
set -euo pipefail
unset CDPATH   # a set CDPATH makes `cd` print the directory

REPO=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
SHA=$(git -C "$REPO" rev-parse --short HEAD)
BIN=/opt/lel/current/bin
RUN="sudo -H -u orch-runner"   # orch-runner cannot enter baileyrd's directories: every call below runs from /

# 1. Build (no sudo) and install the binaries
cd "$REPO"
cargo build --release -p rusty_bbp_host -p rusty_orch
sudo install -d -m 0755 "/opt/lel/$SHA/bin"
sudo install -m 0755 target/release/{bbp,codex-bbp-harness,claude-bbp-harness,rusty_orch} "/opt/lel/$SHA/bin/"
sudo ln -sfn "$SHA" /opt/lel/current
sudo restorecon -R /opt/lel
cd /

# 2. BBP directory and orch state: orch-runner only.
# The policy default for /srv/lel/* on BaileyAi is container_file_t (a local rule, see
# matchpathcon), so new directories inherit the right label; restorecon would print
# "not reset as customized by admin" and change nothing, so it is not run here.
sudo install -d -o orch-runner -g orch-runner -m 0700 /srv/lel/bbp /srv/lel/orch-state

# 3. Lens configs from baileyrd/Factory main, cloned to a fresh directory
FACT=$(mktemp -d /tmp/factory-lens.XXXXXX)
trap 'rm -rf "$FACT"' EXIT
git clone --depth 1 https://github.com/baileyrd/factory "$FACT"
echo "Factory commit: $(git -C "$FACT" rev-parse --short HEAD)"
sudo install -d -o orch-runner -g orch-runner -m 0750 /srv/lel/lenses
sudo install -o orch-runner -g orch-runner -m 0640 "$FACT"/lenses/*.md /srv/lel/lenses/
sudo install -o orch-runner -g orch-runner -m 0640 "$FACT/lens.json" "$FACT/lens-local.json" /srv/lel/
# Installed files must match the clone byte for byte:
( cd "$FACT" && sha256sum lens.json lens-local.json lenses/*.md ) > "$FACT.sums"
$RUN sh -c 'cd /srv/lel && sha256sum -c' < "$FACT.sums"
rm -f "$FACT.sums"

# 4. Checks, all as orch-runner from /
echo "--- lens check: local, local-high, local-rebel ready; codex ready only inside lel-codex; claude unavailable until its assertion exists"
$RUN $BIN/rusty_orch lens check /srv/lel/lens.json --repo /srv/lel/experiment
$RUN $BIN/rusty_orch lens check /srv/lel/lens-local.json --repo /srv/lel/experiment
echo "--- bbp answers (usage line expected; it needs a verb and --dir)"
$RUN env BBP_DIR=/srv/lel/bbp $BIN/bbp 2>&1 | head -1 || true
echo "--- /srv/lel/bbp writable by orch-runner, refused to baileyrd"
$RUN touch /srv/lel/bbp/.w && $RUN rm /srv/lel/bbp/.w
if ls /srv/lel/bbp >/dev/null 2>&1; then echo "FAIL: baileyrd can read /srv/lel/bbp"; exit 1; else echo "ok: refused"; fi
echo "--- labels (policy default vs actual)"
matchpathcon /srv/lel/bbp /srv/lel/orch-state /srv/lel/lenses /srv/lel/lens.json /srv/lel/lens-local.json
ls -dZ /srv/lel/bbp /srv/lel/orch-state /srv/lel/lenses
echo "--- lel-codex untouched"
$RUN podman ps -a --format '{{.Names}} {{.Status}}'
echo "--- AVC denials since the script started (none expected)"
sudo ausearch -m avc -ts recent 2>/dev/null | tail -5 || true

#!/usr/bin/env bash
# Tear down the interop environment started by up.sh.
set -uo pipefail
cd "$(dirname "$0")"
for n in 1 2; do
  pidfile="state/ts$n/tailscaled.pid"
  [ -f "$pidfile" ] && kill "$(cat "$pidfile")" 2>/dev/null
  rm -f "$pidfile"
done
docker rm -f ts-rs-headscale >/dev/null 2>&1
echo "interop environment stopped (state/ left in place; delete to reset)"

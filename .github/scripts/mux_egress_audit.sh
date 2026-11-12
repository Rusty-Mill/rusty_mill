#!/usr/bin/env bash
# C09 egress audit (card #266): runs the mux server, ctl, mcp and TUI through a
# scripted session under strace and fails if any of them opens a network
# socket (anything but AF_UNIX) or connects/sends anywhere but a Unix socket.
# The block's own shell is the user's program and is traced too: the session
# only runs `echo`, so it must stay quiet as well.
# Usage: .github/scripts/mux_egress_audit.sh [TARGET_DIR]   (default target/release)
set -euo pipefail
bin="${1:-target/release}"
for b in rusty-mux-server rusty-mux-ctl rusty-mux-mcp rusty-mux; do
  [[ -x "$bin/$b" ]] || { echo "missing $bin/$b; build it first" >&2; exit 2; }
done
command -v strace >/dev/null || { echo "strace is required" >&2; exit 2; }
work="$(mktemp -d)"
trap 'kill "$server" 2>/dev/null || true; rm -rf "$work"' EXIT
sock="$work/mux.sock"
export HOME="$work/home" XDG_CONFIG_HOME="$work/config" XDG_RUNTIME_DIR="$work/run"
mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_RUNTIME_DIR"
trace=(strace -f -qq -e trace=socket,connect,sendto,sendmsg,bind -o)

"${trace[@]}" "$work/server.trace" "$bin/rusty-mux-server" "$sock" 2>"$work/server.err" &
server=$!
for _ in $(seq 50); do [[ -S "$sock" ]] && break; sleep 0.1; done
[[ -S "$sock" ]] || { cat "$work/server.err" >&2; echo "server did not start" >&2; exit 1; }

ctl() { timeout 20 "${trace[@]}" "$work/ctl.$1.trace" "$bin/rusty-mux-ctl" --socket "$sock" "${@:2}" >/dev/null; }
ctl 1 create-session --name audit
ctl 2 create-window --session audit --name w
ctl 3 create-block --window audit/w --cols 80 --rows 24 --command /bin/echo --arg egress-audit
ctl 4 list
ctl 5 whoami
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"audit","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' |
  RUSTY_MUX_SOCKET="$sock" timeout 20 "${trace[@]}" "$work/mcp.trace" "$bin/rusty-mux-mcp" >/dev/null || true
# The TUI attaches through a pty for a few seconds, then is stopped.
if command -v script >/dev/null; then
  script -qec "timeout -s TERM 3 strace -f -qq -e trace=socket,connect,sendto,sendmsg,bind -o '$work/tui.trace' '$bin/rusty-mux' '$sock'" /dev/null >/dev/null 2>&1 </dev/null || true
  [[ -s "$work/tui.trace" ]] || { echo "the TUI did not run" >&2; exit 1; }
fi
# Stop the traced server itself (strace's child), then let strace finish.
pkill -INT -P "$server" || true
timeout 20 tail --pid="$server" -f /dev/null || kill "$server" || true

# Network-family sockets, and connect/send to a non-Unix address.
bad="$(grep -hE 'socket\((AF_INET|AF_INET6|AF_PACKET|AF_NETLINK)|(connect|sendto|sendmsg|bind)\([0-9]+, \{sa_family=AF_(INET|INET6)' "$work"/*.trace || true)"
calls="$(cat "$work"/*.trace | grep -cE 'socket\(|connect\(' || true)"
if [[ -n "$bad" ]]; then
  echo "egress audit: FAIL, network calls found:" >&2
  echo "$bad" >&2
  exit 1
fi
echo "egress audit: PASS ($calls socket/connect calls traced, all AF_UNIX)"

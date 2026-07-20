#!/usr/bin/env bash
# Live interoperability test between rusty-croc and stock Go croc.
#
# Proves, against the real Go implementation:
#   1. A stock croc client pair transfers a file through the rusty-croc relay.
#   2. The rusty-croc relay-client handshake works against the stock Go relay.
#
# Requirements: cargo, go, git, network access to github.com (to fetch croc).
# Override CROC_SRC to point at an existing croc checkout.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="${TMPDIR:-/tmp}/rusty-croc-interop"
CROC_SRC="${CROC_SRC:-$WORK/croc}"
RELAY_PORTS="9119,9120,9121,9122,9123"
PASS="pass123"
SECRET="$((1000 + RANDOM % 9000))-interop-test-phrase"

mkdir -p "$WORK"
cleanup() { kill $(jobs -p) 2>/dev/null || true; }
trap cleanup EXIT

echo "==> building rusty-croc"
cargo build --release --manifest-path "$ROOT/Cargo.toml" --bin rusty-croc
cargo build --release --manifest-path "$ROOT/Cargo.toml" --example join_room
RUSTY="$ROOT/target/release/rusty-croc"

echo "==> building stock Go croc"
if [ ! -d "$CROC_SRC" ]; then
  git clone --depth 1 https://github.com/schollz/croc "$CROC_SRC"
fi
(cd "$CROC_SRC" && go build -o "$WORK/croc-go" .)
CROC="$WORK/croc-go"

echo "==> test 1: stock croc file transfer through the rusty-croc relay"
"$RUSTY" relay --ports "$RELAY_PORTS" --pass "$PASS" &>"$WORK/rust-relay.log" &
for i in $(seq 1 50); do
  "$RUSTY" ping 127.0.0.1:9119 &>/dev/null && break
  sleep 0.1
done

head -c 1048576 /dev/urandom > "$WORK/sendfile.bin"
rm -rf "$WORK/recv" && mkdir -p "$WORK/recv"

(cd "$WORK" && CROC_SECRET="$SECRET" "$CROC" --relay 127.0.0.1:9119 --pass "$PASS" \
  send sendfile.bin &>"$WORK/send.log") &
SEND_PID=$!
sleep 2
(cd "$WORK/recv" && CROC_SECRET="$SECRET" "$CROC" --yes --relay 127.0.0.1:9119 \
  --pass "$PASS" &>"$WORK/recv.log")
wait "$SEND_PID"

if cmp -s "$WORK/sendfile.bin" "$WORK/recv/sendfile.bin"; then
  echo "    OK: file transferred through Rust relay, checksums match"
else
  echo "    FAIL: transferred file differs (see $WORK/*.log)"; exit 1
fi

echo "==> test 2: rusty-croc client handshake against the stock Go relay"
"$CROC" relay --ports 9130,9131 &>"$WORK/go-relay.log" &
sleep 1.5
OUT="$("$ROOT/target/release/examples/join_room" 127.0.0.1:9130 "$PASS" interop-room)"
echo "    OK: $OUT"

# --- phase 2: file-transfer engine interop --------------------------------

transfer_case() {
  # transfer_case <name> <sender-cmd...> -- <receiver-cmd...>
  local name="$1"; shift
  local code="$((1000 + RANDOM % 9000))-interop-case-$name"
  local src="$WORK/payload-$name.bin"
  head -c 2000000 /dev/urandom > "$src"
  rm -rf "$WORK/out-$name" && mkdir -p "$WORK/out-$name"

  local sender=() receiver=()
  local mode=sender
  for arg in "$@"; do
    if [ "$arg" = "--" ]; then mode=receiver; continue; fi
    if [ "$mode" = "sender" ]; then sender+=("$arg"); else receiver+=("$arg"); fi
  done

  (CROC_SECRET="$code" timeout 120 "${sender[@]}" "$src" &>"$WORK/send-$name.log") &
  local spid=$!
  sleep 2
  (cd "$WORK/out-$name" && CROC_SECRET="$code" timeout 90 "${receiver[@]}" &>"$WORK/recv-$name.log")
  wait "$spid"
  if cmp -s "$src" "$WORK/out-$name/$(basename "$src")"; then
    echo "    OK: $name"
  else
    echo "    FAIL: $name (see $WORK/{send,recv}-$name.log)"; exit 1
  fi
}

echo "==> test 3: transfer matrix through the rusty-croc relay"
transfer_case rust-to-rust \
  "$RUSTY" send --relay 127.0.0.1:9119 --pass "$PASS" -- \
  "$RUSTY" receive --yes --relay 127.0.0.1:9119 --pass "$PASS"
transfer_case rust-to-go \
  "$RUSTY" send --relay 127.0.0.1:9119 --pass "$PASS" -- \
  "$CROC" --yes --relay 127.0.0.1:9119 --pass "$PASS"
transfer_case go-to-rust \
  "$CROC" --relay 127.0.0.1:9119 --pass "$PASS" send -- \
  "$RUSTY" receive --yes --relay 127.0.0.1:9119 --pass "$PASS"

echo "==> all interop tests passed"

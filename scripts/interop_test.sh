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
RELAY_PORTS="9019,9020,9021,9022,9023"
PASS="pass123"
SECRET="8888-interop-test-phrase"

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
  "$RUSTY" ping 127.0.0.1:9019 &>/dev/null && break
  sleep 0.1
done

head -c 1048576 /dev/urandom > "$WORK/sendfile.bin"
rm -rf "$WORK/recv" && mkdir -p "$WORK/recv"

(cd "$WORK" && CROC_SECRET="$SECRET" "$CROC" --relay 127.0.0.1:9019 --pass "$PASS" \
  send sendfile.bin &>"$WORK/send.log") &
SEND_PID=$!
sleep 2
(cd "$WORK/recv" && CROC_SECRET="$SECRET" "$CROC" --yes --relay 127.0.0.1:9019 \
  --pass "$PASS" &>"$WORK/recv.log")
wait "$SEND_PID"

if cmp -s "$WORK/sendfile.bin" "$WORK/recv/sendfile.bin"; then
  echo "    OK: file transferred through Rust relay, checksums match"
else
  echo "    FAIL: transferred file differs (see $WORK/*.log)"; exit 1
fi

echo "==> test 2: rusty-croc client handshake against the stock Go relay"
"$CROC" relay --ports 9030,9031 &>"$WORK/go-relay.log" &
sleep 1.5
OUT="$("$ROOT/target/release/examples/join_room" 127.0.0.1:9030 "$PASS" interop-room)"
echo "    OK: $OUT"

echo "==> all interop tests passed"

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

echo "==> test 4: local-network route via ips? probe (recipient hops to sender's local relay)"
code="$((1000 + RANDOM % 9000))-interop-local-route"
head -c 500000 /dev/urandom > "$WORK/payload-local.bin"
rm -rf "$WORK/out-local" && mkdir -p "$WORK/out-local"
(CROC_SECRET="$code" timeout 120 "$RUSTY" --debug send --relay 127.0.0.1:9119 --pass "$PASS" \
  "$WORK/payload-local.bin" &>"$WORK/send-local.log") &
spid=$!
sleep 2
(cd "$WORK/out-local" && CROC_SECRET="$code" timeout 90 "$CROC" --debug --yes \
  --relay 127.0.0.1:9119 --pass "$PASS" &>"$WORK/recv-local.log")
wait "$spid"
if cmp -s "$WORK/payload-local.bin" "$WORK/out-local/payload-local.bin" \
   && grep -q "local connection established" "$WORK/recv-local.log"; then
  echo "    OK: transfer went through the rusty-croc sender's local relay"
else
  echo "    FAIL: local route (see $WORK/{send,recv}-local.log)"; exit 1
fi

echo "==> test 5: --text both directions"
code="$((1000 + RANDOM % 9000))-interop-text-a"
(CROC_SECRET="$code" timeout 60 "$RUSTY" send --text "text from rust" --no-local \
  --relay 127.0.0.1:9119 --pass "$PASS" &>"$WORK/send-text-a.log") &
spid=$!
sleep 2
OUT="$(cd "$WORK" && CROC_SECRET="$code" timeout 60 "$CROC" --yes --relay 127.0.0.1:9119 --pass "$PASS" 2>/dev/null)"
wait "$spid"
[ "$OUT" = "text from rust" ] && echo "    OK: rust --text → go" || { echo "    FAIL: rust --text → go ('$OUT')"; exit 1; }

code="$((1000 + RANDOM % 9000))-interop-text-b"
(cd "$WORK" && CROC_SECRET="$code" timeout 60 "$CROC" --relay 127.0.0.1:9119 --pass "$PASS" \
  send --no-local --text "text from go" &>"$WORK/send-text-b.log") &
spid=$!
sleep 2
OUT="$(cd "$WORK" && CROC_SECRET="$code" timeout 60 "$RUSTY" receive --yes --no-local --relay 127.0.0.1:9119 --pass "$PASS" 2>/dev/null)"
wait "$spid"
[ "$OUT" = "text from go" ] && echo "    OK: go --text → rust" || { echo "    FAIL: go --text → rust ('$OUT')"; exit 1; }

echo "==> test 6: --zip folder go → rust (received archive is unpacked)"
code="$((1000 + RANDOM % 9000))-interop-zip-case"
rm -rf "$WORK/zipsrc" "$WORK/out-zip" && mkdir -p "$WORK/zipsrc/sub" "$WORK/out-zip"
echo "one" > "$WORK/zipsrc/a.txt"
head -c 200000 /dev/urandom > "$WORK/zipsrc/sub/b.bin"
(cd "$WORK" && CROC_SECRET="$code" timeout 90 "$CROC" --relay 127.0.0.1:9119 --pass "$PASS" \
  send --no-local --zip zipsrc &>"$WORK/send-zip.log") &
spid=$!
sleep 2
(cd "$WORK/out-zip" && CROC_SECRET="$code" timeout 90 "$RUSTY" receive --yes --no-local \
  --relay 127.0.0.1:9119 --pass "$PASS" &>"$WORK/recv-zip.log")
wait "$spid"
if diff -r "$WORK/zipsrc" "$WORK/out-zip/zipsrc" >/dev/null && [ ! -e "$WORK/out-zip/zipsrc.zip" ]; then
  echo "    OK: zip transferred and unpacked"
else
  echo "    FAIL: zip case (see $WORK/{send,recv}-zip.log)"; exit 1
fi

echo "==> test 7: reconnect-and-resume through a severed relay (rust send → go recv)"
"$RUSTY" relay --ports 9139,9140,9141,9142,9143 --pass "$PASS" \
  --test-sever-after 1500000 &>"$WORK/sever-relay.log" &
sleep 1
code="$((1000 + RANDOM % 9000))-interop-reconnect"
head -c 6000000 /dev/urandom > "$WORK/payload-rc.bin"
rm -rf "$WORK/out-rc" && mkdir -p "$WORK/out-rc"
(CROC_SECRET="$code" timeout 120 "$RUSTY" send --no-local --relay 127.0.0.1:9139 --pass "$PASS" \
  "$WORK/payload-rc.bin" &>"$WORK/send-rc.log") &
spid=$!
sleep 2
(cd "$WORK/out-rc" && CROC_SECRET="$code" timeout 90 "$CROC" --yes --overwrite \
  --relay 127.0.0.1:9139 --pass "$PASS" &>"$WORK/recv-rc.log")
wait "$spid"
if cmp -s "$WORK/payload-rc.bin" "$WORK/out-rc/payload-rc.bin" \
   && grep -q "interruption" "$WORK/send-rc.log" \
   && grep -q "dropping all piped connections" "$WORK/sever-relay.log"; then
  echo "    OK: relay severed mid-transfer; both sides reconnected and resumed"
else
  echo "    FAIL: reconnect case (see $WORK/{send,recv}-rc.log, sever-relay.log)"; exit 1
fi

echo "==> all interop tests passed"

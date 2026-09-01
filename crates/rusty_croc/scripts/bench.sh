#!/usr/bin/env bash
# Benchmark rusty-croc against stock Go croc, full-stack (each implementation's
# own client + relay), over loopback — so it measures protocol/CPU/IO overhead,
# not the network. Reports best-of-N to cut scheduling noise.
#
# Requirements: cargo, go, git. Override CROC_SRC to reuse a croc checkout.
# Note: loopback numbers isolate CPU/protocol cost; over a real network both
# implementations are network-bound and roughly tie.

set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="${TMPDIR:-/tmp}/rusty-croc-bench"
CROC_SRC="${CROC_SRC:-$WORK/croc}"
PASS=pass123
mkdir -p "$WORK"
cleanup() { pkill -x rusty-croc 2>/dev/null; pkill -x croc-go 2>/dev/null; }
trap cleanup EXIT

echo "==> building binaries"
cargo build --release --manifest-path "$ROOT/Cargo.toml" --bin rusty-croc >/dev/null
RUSTY="$ROOT/target/release/rusty-croc"
if [ ! -d "$CROC_SRC" ]; then git clone --depth 1 https://github.com/schollz/croc "$CROC_SRC" >/dev/null 2>&1; fi
( cd "$CROC_SRC" && go build -o "$WORK/croc-go" . )
GO="$WORK/croc-go"

now() { date +%s.%N; }
minv() { printf '%s\n' "$@" | sort -n | head -1; }
# Peak VmRSS (KB) of a pid until it exits.
sample_rss() {
  local pid=$1 out=$2 max=0 r
  while [ -d "/proc/$pid" ]; do
    r=$(awk '/VmRSS/{print $2}' "/proc/$pid/status" 2>/dev/null)
    [ -n "$r" ] && [ "$r" -gt "$max" ] && max=$r
    sleep 0.02
  done
  echo "$max" > "$out"
}

# Distinct relay ports per implementation so restarts never collide.
RUST_PORTS=9909,9910,9911,9912,9913; RUST_RELAY=127.0.0.1:9909
GO_PORTS=9919,9920,9921,9922,9923;   GO_RELAY=127.0.0.1:9919

# run_once <impl> <size_mb> <compress 0/1> <idx> -> "seconds rss_kb"
run_once() {
  local impl=$1 mb=$2 comp=$3 idx=$4
  local code="bench-$impl-$mb-$comp-$idx-xyz"
  local src="$WORK/src_$mb.bin" dst="$WORK/out"
  rm -rf "$dst"; mkdir -p "$dst"
  local sflags=""; [ "$comp" = 0 ] && sflags="--no-compress"
  local relay=$RUST_RELAY; [ "$impl" = go ] && relay=$GO_RELAY

  if [ "$impl" = rust ]; then
    CROC_SECRET="$code" "$RUSTY" send $sflags --no-local --relay "$relay" --pass "$PASS" "$src" >/dev/null 2>&1 &
  else
    CROC_SECRET="$code" "$GO" $sflags --relay "$relay" --pass "$PASS" send --no-local "$src" >/dev/null 2>&1 &
  fi
  local sp=$!; sleep 2

  local t0 t1 rp
  t0=$(now)
  if [ "$impl" = rust ]; then
    CROC_SECRET="$code" env -C "$dst" "$RUSTY" receive --yes --overwrite --no-local --relay "$relay" --pass "$PASS" >/dev/null 2>&1 &
  else
    CROC_SECRET="$code" env -C "$dst" "$GO" --yes --overwrite --relay "$relay" --pass "$PASS" >/dev/null 2>&1 &
  fi
  rp=$!
  local rssf="$WORK/rss"; sample_rss "$rp" "$rssf" &
  wait "$rp"; t1=$(now); wait "$sp" 2>/dev/null
  cmp -s "$src" "$dst/src_$mb.bin" || echo "  !! MISMATCH $impl $mb comp=$comp" >&2
  rm -rf "$dst"
  awk -v a="$t0" -v b="$t1" -v r="$(cat "$rssf" 2>/dev/null || echo 0)" 'BEGIN{printf "%.3f %d\n", b-a, r}'
}

bench() { # impl size_mb compress trials
  local impl=$1 mb=$2 comp=$3 trials=$4 ports=$RUST_PORTS
  [ "$impl" = go ] && ports=$GO_PORTS
  if [ "$impl" = rust ]; then "$RUSTY" relay --ports "$ports" --pass "$PASS" >/dev/null 2>&1 &
  else "$GO" relay --ports "$ports" >/dev/null 2>&1 & fi
  local rp=$!; sleep 1
  local times=() rsses=()
  for i in $(seq 1 "$trials"); do
    read -r t r < <(run_once "$impl" "$mb" "$comp" "$i")
    times+=("$t"); rsses+=("$r")
  done
  kill "$rp" 2>/dev/null; sleep 0.5
  local bt br mbps rmb
  bt=$(minv "${times[@]}"); br=$(minv "${rsses[@]}")
  mbps=$(awk -v s="$bt" -v mb="$mb" 'BEGIN{ if (mb>0 && s>0) printf "%.0f", mb/s; else printf "-" }')
  rmb=$(awk -v r="$br" 'BEGIN{printf "%.0f", r/1024}')
  printf "  %-4s  best %6ss   %5s MB/s   peakRSS %3s MB\n" "$impl" "$bt" "$mbps" "$rmb"
}

echo
echo "=== rusty-croc vs Go croc — full-stack loopback (best of N) ==="

echo
echo "[handshake latency]  64 KiB transfer (dominated by PAKE + setup), best of 5"
head -c 65536 /dev/urandom > "$WORK/src_0.bin"
for impl in rust go; do bench "$impl" 0 1 5; done

head -c $((200*1024*1024)) /dev/urandom > "$WORK/src_200.bin"
echo
echo "[throughput]  200 MB random, compression ON (default), best of 3"
for impl in rust go; do bench "$impl" 200 1 3; done
echo
echo "[throughput]  200 MB random, --no-compress (raw crypto+framing+IO), best of 3"
for impl in rust go; do bench "$impl" 200 0 3; done
echo
echo "[throughput]  200 MB zeros (compressible), compression ON, best of 3"
head -c $((200*1024*1024)) /dev/zero > "$WORK/src_200.bin"
for impl in rust go; do bench "$impl" 200 1 3; done

rm -f "$WORK"/src_*.bin
echo
printf "binary size: rusty-croc %s | croc-go %s\n" \
  "$(stat -c%s "$RUSTY" | awk '{printf "%.1f MB", $1/1048576}')" \
  "$(stat -c%s "$GO"    | awk '{printf "%.1f MB", $1/1048576}')"

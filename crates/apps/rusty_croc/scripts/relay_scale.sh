#!/usr/bin/env bash
# Evaluate the rusty-croc relay's concurrency: drive N simultaneous transfers
# through a single relay and report success rate, wall-clock, aggregate
# throughput, and the relay's peak thread count / RSS. Loopback, so this
# measures the relay's own overhead, not the network.
#
# The relay is thread-per-connection (a control thread per client, plus two
# pipe threads per stapled room). This script characterizes how that scales.
#
# Usage: [CONCURRENCY=50] [SIZE_MB=4] scripts/relay_scale.sh

set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUSTY="$ROOT/target/release/rusty-croc"
WORK="${TMPDIR:-/tmp}/rusty-croc-scale"
PASS=pass123
C="${CONCURRENCY:-50}"
MB="${SIZE_MB:-4}"
mkdir -p "$WORK"
cleanup() { pkill -x rusty-croc 2>/dev/null; }
trap cleanup EXIT

cargo build --release --manifest-path "$ROOT/Cargo.toml" --bin rusty-croc >/dev/null 2>&1 || {
  echo "build failed"; exit 1; }

head -c $((MB * 1024 * 1024)) /dev/urandom > "$WORK/payload.bin"

echo "=== relay concurrency: $C simultaneous ${MB}MB transfers, one relay ==="
"$RUSTY" relay --ports 9909,9910,9911,9912,9913 --pass "$PASS" >/dev/null 2>&1 &
RELAY=$!
sleep 1

# Peak thread-count / RSS sampler for the relay.
peakfile="$WORK/peak"; echo "0 0" > "$peakfile"
( maxthreads=0; maxrss=0
  while [ -d "/proc/$RELAY" ]; do
    t=$(awk '/Threads/{print $2}' "/proc/$RELAY/status" 2>/dev/null)
    r=$(awk '/VmRSS/{print $2}' "/proc/$RELAY/status" 2>/dev/null)
    [ -n "$t" ] && [ "$t" -gt "$maxthreads" ] && maxthreads=$t
    [ -n "$r" ] && [ "$r" -gt "$maxrss" ] && maxrss=$r
    echo "$maxthreads $maxrss" > "$peakfile"
    sleep 0.05
  done ) &

start=$(date +%s.%N)
pids=()
for i in $(seq 1 "$C"); do
  # croc derives the relay room from the code's first 4 chars, so the prefix
  # must be unique per transfer or they all collide in one room.
  code="$(printf %04d "$i")-scale-xyz"
  out="$WORK/out-$i"; rm -rf "$out"; mkdir -p "$out"
  ( CROC_SECRET="$code" "$RUSTY" send --no-local --relay 127.0.0.1:9909 --pass "$PASS" \
      "$WORK/payload.bin" >/dev/null 2>&1 ) &
  ( sleep 1
    cd "$out" && CROC_SECRET="$code" timeout 120 "$RUSTY" receive --yes --overwrite \
      --no-local --relay 127.0.0.1:9909 --pass "$PASS" >/dev/null 2>&1 ) &
  pids+=($!)
done
# Wait for all receivers.
for p in "${pids[@]}"; do wait "$p"; done
end=$(date +%s.%N)

ok=0
for i in $(seq 1 "$C"); do
  cmp -s "$WORK/payload.bin" "$WORK/out-$i/payload.bin" && ok=$((ok + 1))
  rm -rf "$WORK/out-$i"
done

read -r peakthreads peakrss < "$peakfile"
wall=$(awk -v a="$start" -v b="$end" 'BEGIN{printf "%.2f", b-a}')
agg=$(awk -v ok="$ok" -v mb="$MB" -v s="$wall" 'BEGIN{ if (s>0) printf "%.0f", ok*mb/s; else printf "-" }')
rssmb=$(awk -v r="$peakrss" 'BEGIN{printf "%.0f", r/1024}')

echo
printf "  transfers succeeded : %d / %d\n" "$ok" "$C"
printf "  wall clock          : %s s\n" "$wall"
printf "  aggregate throughput: %s MB/s\n" "$agg"
printf "  relay peak threads  : %s\n" "$peakthreads"
printf "  relay peak RSS      : %s MB\n" "$rssmb"
rm -f "$WORK/payload.bin"

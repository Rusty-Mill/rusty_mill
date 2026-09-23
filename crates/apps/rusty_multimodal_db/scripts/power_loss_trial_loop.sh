#!/usr/bin/env bash
# Power-loss trial without device-mapper (ADR-0119 follow-up): the
# crash-prefix snapshot variant for a host that has loop devices and
# root but no dm-log-writes (a Firecracker guest, most containers).
#
# The store lives on ext4 over a loop device opened with --direct-io=on,
# so the backing file receives exactly the bytes the filesystem pushed
# to the "device": what fsync/msync forced, plus whatever background
# writeback had flushed. At the chosen sync point the writer is
# SIGKILLed and the backing file is copied at once — before the kernel's
# dirty-page expiry (30 s by default) can write anything more back — so
# the copy is a crash prefix: the device state a power loss at that
# instant would have left, page cache and all lost. The copy is mounted
# on a second loop device (ext4 replays its journal, as after a real
# power loss) and reopened through `crash_writer reopen-check`.
#
# Weaker than scripts/power_loss_trial.sh (one prefix per run, not a
# replay to every mark; block reordering within the device is not
# modelled), stronger than STORAGE-021's SIGKILL gate (the page cache
# is not consulted). Root required; every device is torn down on exit.
#
# Usage: sudo scripts/power_loss_trial_loop.sh [flushed|torn-write|unflushed] [count]
set -euo pipefail

MODE="${1:-flushed}"
COUNT="${2:-500}"
SIZE_MB=128
HERE="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$(mktemp -d /tmp/rmdb-power-loss-loop.XXXXXX)"
LOOP=""; SNAP_LOOP=""
MOUNT="$WORK/mnt"; SNAP_MOUNT="$WORK/snap"
WRITER_PID=""

refuse() { echo "power_loss_trial_loop: $*" >&2; exit 2; }
[ "$(id -u)" -eq 0 ] || refuse "root is required (loop devices and mounts)"
command -v losetup >/dev/null || refuse "losetup not found"
command -v mkfs.ext4 >/dev/null || refuse "mkfs.ext4 not found"
[ -e /dev/loop-control ] || refuse "no /dev/loop-control"

cleanup() {
  set +e
  [ -n "$WRITER_PID" ] && kill -9 "$WRITER_PID" 2>/dev/null
  mountpoint -q "$SNAP_MOUNT" 2>/dev/null && umount "$SNAP_MOUNT"
  mountpoint -q "$MOUNT" 2>/dev/null && umount "$MOUNT"
  [ -n "$SNAP_LOOP" ] && losetup -d "$SNAP_LOOP"
  [ -n "$LOOP" ] && losetup -d "$LOOP"
  rm -rf "$WORK"
}
trap cleanup EXIT

echo "building crash_writer (research)"
( cd "$HERE" && cargo build --release --features research --bin crash_writer >/dev/null 2>&1 )
TARGET_DIR="$(cd "$HERE" && cargo metadata --format-version 1 2>/dev/null | python3 -c 'import sys,json;print(json.load(sys.stdin)["target_directory"])')"
WRITER="$TARGET_DIR/release/crash_writer"
[ -x "$WRITER" ] || refuse "crash_writer did not build at $WRITER"

truncate -s "${SIZE_MB}M" "$WORK/data.img"
LOOP="$(losetup --find --show --direct-io=on "$WORK/data.img")"
mkfs.ext4 -q "$LOOP"
mkdir -p "$MOUNT" "$SNAP_MOUNT"
mount "$LOOP" "$MOUNT"
STORE="$MOUNT/trial.mmap"

wait_line() {
  until grep -q "^$1\$" "$WORK/writer.out" 2>/dev/null; do
    kill -0 "$WRITER_PID" 2>/dev/null || refuse "crash_writer exited before printing $1: $(tail -3 "$WORK/writer.out")"
    sleep 0.02
  done
}
snapshot() {
  # The crash instant: kill, then copy the device before writeback runs.
  kill -9 "$WRITER_PID" 2>/dev/null || true
  cp "$WORK/data.img" "$WORK/snap-$1.img"
  echo "snapshot at $1 taken $(date +%T.%N | cut -c1-12)"
}

echo "running crash_writer $MODE against $STORE"
case "$MODE" in
  flushed)
    "$WRITER" flushed-updates "$STORE" "$COUNT" > "$WORK/writer.out" & WRITER_PID=$!
    wait_line FLUSHED; snapshot flushed; MARKS="flushed" ;;
  unflushed)
    "$WRITER" unflushed-updates "$STORE" "$COUNT" > "$WORK/writer.out" & WRITER_PID=$!
    wait_line ALL_DONE; snapshot all_done; MARKS="all_done" ;;
  torn-write)
    # Seed three durable records (create + Flush), then append a fourth
    # slot and cut after its value, before its commit marker: the reopen
    # must keep the three and exclude the torn one.
    "$WRITER" flushed-updates "$STORE" 3 > "$WORK/writer.out" & WRITER_PID=$!
    wait_line FLUSHED; kill -9 "$WRITER_PID" 2>/dev/null || true; wait "$WRITER_PID" 2>/dev/null || true
    "$WRITER" torn-write "$STORE" 3 4 42 > "$WORK/writer.out" & WRITER_PID=$!
    wait_line VALUE_WRITTEN; snapshot value_written; MARKS="value_written" ;;
  *) refuse "unknown mode $MODE (flushed|torn-write|unflushed)" ;;
esac
wait "$WRITER_PID" 2>/dev/null || true; WRITER_PID=""
umount "$MOUNT"

STATUS=0
for MARK in $MARKS; do
  # Mounted read-write: the copy is disposable, and ext4 must replay its
  # journal at mount exactly as it would after a real power loss.
  SNAP_LOOP="$(losetup --find --show "$WORK/snap-$MARK.img")"
  mount "$SNAP_LOOP" "$SNAP_MOUNT"
  if OUT="$("$WRITER" reopen-check "$SNAP_MOUNT/trial.mmap" 2>&1)"; then
    echo "mark $MARK: $OUT"
  else
    echo "mark $MARK: FAILED — $OUT"; STATUS=1
  fi
  umount "$SNAP_MOUNT"; losetup -d "$SNAP_LOOP"; SNAP_LOOP=""
done
exit $STATUS

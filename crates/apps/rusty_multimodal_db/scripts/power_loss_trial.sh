#!/usr/bin/env bash
# Power-loss trial for rusty_multimodal_db's storage (ADR-0119,
# docs/design/STORAGE-POWER-LOSS-DESIGN.md, PLP-FR-003).
#
# Records every block write the crash_writer makes through a
# dm-log-writes device, then replays the device to each flush mark the
# writer's sync points left and reopens the store there, so what is
# checked is the bytes that would have reached the device at a power
# loss — not the page cache a SIGKILL leaves intact (STORAGE-021's gate).
#
# Requires: Linux with the dm-log-writes module, `replay-log` from
# xfstests (src/log-writes), losetup, dmsetup, mkfs.ext4, cargo. Root.
# Runs on a disposable host only; every device it creates is removed on
# exit. Not run in CI (no root device-mapper access there).
#
# Usage: sudo scripts/power_loss_trial.sh [flushed|torn-write|unflushed] [count]
set -euo pipefail

MODE="${1:-flushed}"
COUNT="${2:-500}"
SIZE_MB=256
HERE="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$(mktemp -d /tmp/rmdb-power-loss.XXXXXX)"
LOOP=""
LOG_LOOP=""
DM_NAME="rmdb-log-writes-$$"
WRITER_PID=""
MOUNT="$WORK/mnt"

refuse() { echo "power_loss_trial: $*" >&2; exit 2; }

[ "$(id -u)" -eq 0 ] || refuse "root is required (device-mapper and loop devices)"
command -v dmsetup >/dev/null || refuse "dmsetup not found"
command -v losetup >/dev/null || refuse "losetup not found"
command -v replay-log >/dev/null || refuse "replay-log (xfstests src/log-writes) not found on PATH"
command -v mkfs.ext4 >/dev/null || refuse "mkfs.ext4 not found"
modprobe dm-log-writes 2>/dev/null || refuse "the dm-log-writes module is not available"

cleanup() {
  set +e
  [ -n "$WRITER_PID" ] && kill -9 "$WRITER_PID" 2>/dev/null
  mountpoint -q "$MOUNT" && umount "$MOUNT"
  [ -n "$DM_NAME" ] && dmsetup info "$DM_NAME" >/dev/null 2>&1 && dmsetup remove "$DM_NAME"
  [ -n "$LOOP" ] && losetup -d "$LOOP"
  [ -n "$LOG_LOOP" ] && losetup -d "$LOG_LOOP"
  rm -rf "$WORK"
}
trap cleanup EXIT

echo "building crash_writer and the replay checker"
( cd "$HERE" && cargo build --release --features research --bin crash_writer >/dev/null )
WRITER="$HERE/../../../target/release/crash_writer"
[ -x "$WRITER" ] || WRITER="$(cd "$HERE" && cargo metadata --format-version 1 | python3 -c 'import sys,json;print(json.load(sys.stdin)["target_directory"])')/release/crash_writer"
[ -x "$WRITER" ] || refuse "crash_writer did not build at $WRITER"

echo "creating a ${SIZE_MB} MiB data device and a log device"
truncate -s "${SIZE_MB}M" "$WORK/data.img"
truncate -s "$((SIZE_MB * 4))M" "$WORK/log.img"
LOOP="$(losetup --find --show "$WORK/data.img")"
LOG_LOOP="$(losetup --find --show "$WORK/log.img")"
SECTORS="$(blockdev --getsz "$LOOP")"
dmsetup create "$DM_NAME" --table "0 $SECTORS log-writes $LOOP $LOG_LOOP"
DEV="/dev/mapper/$DM_NAME"
mkfs.ext4 -q "$DEV"
mkdir -p "$MOUNT"
mount "$DEV" "$MOUNT"
dmsetup message "$DM_NAME" 0 mark mkfs

STORE="$MOUNT/trial.mmap"
MARKS=""
# Waits for the writer's line; a writer that died first is a failed trial,
# not a hang.
wait_line() {
  until grep -q "^$1\$" "$WORK/writer.out" 2>/dev/null; do
    kill -0 "$WRITER_PID" 2>/dev/null || refuse "crash_writer exited before printing $1: $(tail -3 "$WORK/writer.out")"
    sleep 0.02
  done
}
echo "running crash_writer $MODE against $STORE"
case "$MODE" in
  flushed)
    "$WRITER" flushed-updates "$STORE" "$COUNT" > "$WORK/writer.out" &
    WRITER_PID=$!
    wait_line FLUSHED
    dmsetup message "$DM_NAME" 0 mark flushed
    MARKS="flushed"
    ;;
  unflushed)
    "$WRITER" unflushed-updates "$STORE" "$COUNT" > "$WORK/writer.out" &
    WRITER_PID=$!
    wait_line ALL_DONE
    dmsetup message "$DM_NAME" 0 mark all_done
    MARKS="all_done"
    ;;
  torn-write)
    # Seed three durable records (create + Flush), then append a fourth
    # slot and cut after its id and its value, before its commit marker:
    # the reopen must keep the three and exclude the torn one.
    "$WRITER" flushed-updates "$STORE" 3 > "$WORK/writer.out" & WRITER_PID=$!
    wait_line FLUSHED; kill -9 "$WRITER_PID" 2>/dev/null || true; wait "$WRITER_PID" 2>/dev/null || true
    "$WRITER" torn-write "$STORE" 3 4 42 > "$WORK/writer.out" &
    WRITER_PID=$!
    wait_line ID_WRITTEN
    dmsetup message "$DM_NAME" 0 mark id_written
    wait_line VALUE_WRITTEN
    dmsetup message "$DM_NAME" 0 mark value_written
    wait_line MARKER_WRITTEN
    dmsetup message "$DM_NAME" 0 mark marker_written
    MARKS="id_written value_written marker_written"
    ;;
  *) refuse "unknown mode $MODE (flushed|torn-write|unflushed)";;
esac
kill -9 "$WRITER_PID" 2>/dev/null || true
wait "$WRITER_PID" 2>/dev/null || true
sync
umount "$MOUNT"
dmsetup remove "$DM_NAME"; DM_NAME=""

echo "replaying to each mark and reopening"
# `RGF-FR-002` (ADR-0120): replay-log has no mode that lists marks, so
# the marks are the ones this script set above (MARKS), and the data
# device is zeroed before every replay — replay-log writes the log's
# bios over whatever the device holds, and blocks written after a mark
# (the journal tail sync/umount left) would otherwise survive into the
# "earlier" state.
STATUS=0
for MARK in $MARKS; do
  dd if=/dev/zero of="$LOOP" bs=4M status=none || true
  replay-log --log "$LOG_LOOP" --replay "$LOOP" --end-mark "$MARK" >/dev/null
  # Mounted read-write: ext4 must replay its own journal, as after a cut.
  mount "$LOOP" "$MOUNT"
  if "$WRITER" reopen-check "$STORE" > "$WORK/check-$MARK.out" 2>&1; then
    echo "mark $MARK: $(tail -1 "$WORK/check-$MARK.out")"
  else
    echo "mark $MARK: FAILED — $(tail -1 "$WORK/check-$MARK.out")"; STATUS=1
  fi
  umount "$MOUNT"
done
exit $STATUS

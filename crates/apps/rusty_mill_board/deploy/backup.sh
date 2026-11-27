#!/usr/bin/env bash
# Mill Board backup: a consistent copy of the SQLite store, shipped off the host.
#
#   backup.sh snapshot   online-backup MILL_BOARD_DB into MILL_BOARD_BACKUP_DIR, check it, keep the newest N
#   backup.sh ship       mirror those copies to MILL_BOARD_BACKUP_DEST/<host> (an rsync target)
#   backup.sh            both
#
# The board keeps running: SQLite's online backup API copies a WAL database under a read
# lock, so no stop is needed (a plain cp of the live file is not safe). Settings come from
# board.env (see board.env.example); nothing secret is printed. Needs python3 (stdlib only).
set -euo pipefail
shopt -s inherit_errexit
trap 'echo "mill-board-backup: failed at line $LINENO" >&2' ERR

die() {
    echo "mill-board-backup: $*" >&2
    exit 1
}

db=${MILL_BOARD_DB:?set MILL_BOARD_DB}
dir=${MILL_BOARD_BACKUP_DIR:?set MILL_BOARD_BACKUP_DIR}
keep=${MILL_BOARD_BACKUP_KEEP:-14}
[[ $keep =~ ^[1-9][0-9]*$ ]] || die "MILL_BOARD_BACKUP_KEEP must be a positive whole number, not '$keep'"
umask 077

# Copy $1 to $2 with the online backup API, then open the copy and require integrity_check ok.
copy_and_check() {
    python3 -I - "$1" "$2" <<'PY'
import sqlite3, sys
src_path, dst_path = sys.argv[1:]
src = sqlite3.connect(f"file:{src_path}?mode=ro", uri=True, timeout=30)
dst = sqlite3.connect(dst_path)
src.backup(dst)
src.close()
dst.execute("PRAGMA journal_mode=DELETE")  # a self-contained file, no -wal/-shm beside it
dst.close()
chk = sqlite3.connect(dst_path)
res = chk.execute("PRAGMA integrity_check").fetchall()
if res != [("ok",)]:
    sys.exit(f"integrity_check failed: {res[:3]}")
n = chk.execute("SELECT count(*) FROM sqlite_master").fetchone()[0]
if n == 0:
    sys.exit("copy has no tables")
PY
}

snapshot() {
    [[ -f $db ]] || die "no database at $db"
    mkdir -p "$dir"
    rm -f -- "$dir"/.mill-board-*.partial*
    local name partial
    name="mill-board-$(date -u +%Y%m%dT%H%M%SZ).sqlite3"
    partial="$dir/.$name.partial"
    if ! copy_and_check "$db" "$partial"; then
        rm -f -- "$partial"*
        die "could not write a verified copy of $db"
    fi
    gzip -n -- "$partial"
    mv "$partial.gz" "$dir/$name.gz"
    (cd "$dir" && sha256sum "$name.gz" >"$name.gz.sha256")
    # Keep the newest N; names sort by time.
    local old
    while IFS= read -r old; do
        rm -f -- "$dir/$old" "$dir/$old.sha256"
    done < <(cd "$dir" && printf '%s\n' mill-board-*.sqlite3.gz | sort | head -n "-$keep")
    echo "mill-board-backup: wrote $dir/$name.gz ($(du -h "$dir/$name.gz" | cut -f1)), integrity ok"
}

# How many copies a listing holds.
copies_in() {
    grep -c 'mill-board-.*\.sqlite3\.gz$' || true
}

ship() {
    local base=${MILL_BOARD_BACKUP_DEST:?set MILL_BOARD_BACKUP_DEST: an rsync target off this host}
    # One subdirectory per host, so hosts sharing a destination never prune each other.
    local dest here there
    dest="$base/$(hostname -s)"
    here=$({ compgen -G "$dir/mill-board-*.sqlite3.gz" || true; } | copies_in)
    ((here > 0)) || die "no copy in $dir to ship"
    # Mirror the newest N, unless this host has fewer than it should and fewer than the
    # destination (a wiped local directory), or the destination cannot be listed (down, or
    # not made yet): then add without pruning, and say so.
    local prune=(--delete) listing
    if ! listing=$(rsync --list-only --timeout=60 "$dest/" 2>/dev/null); then
        prune=()
        echo "mill-board-backup: could not list $dest (normal on the first run); not pruning it" >&2
    else
        there=$(copies_in <<<"$listing")
        if ((here < keep && here < there)); then
            prune=()
            echo "mill-board-backup: only $here copies here, $there at the destination; not pruning it" >&2
        fi
    fi
    rsync -a "${prune[@]}" --timeout=600 --include='mill-board-*' --exclude='*' "$dir/" "$dest/"
    echo "mill-board-backup: shipped to $dest"
}

case ${1:-all} in
snapshot) snapshot ;;
ship) ship ;;
all)
    snapshot
    ship
    ;;
*) die "usage: backup.sh [snapshot|ship]" ;;
esac

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

# One run at a time (the timer, or a hand-run snapshot or ship): both rebuild the staging
# directory, and a prune from a staging directory another run emptied would delete the copies.
mkdir -p "$dir"
exec {lock_fd}>"$dir/.lock"
flock -n "$lock_fd" || die "another mill-board-backup is running"

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

# A copy counts only as a verified pair: the gzip, its checksum (published last), and the gzip
# matching it. A stale or damaged gzip is warned about and counted as nothing, so it can neither
# keep a good copy from retention nor authorize pruning. Names of the verified pairs, oldest first.
pairs() {
    local f
    for f in "$dir"/mill-board-*.sqlite3.gz; do
        [[ -f $f && -f $f.sha256 ]] || continue
        if (cd "$dir" && sha256sum -c --status -- "$(basename "$f").sha256" 2>/dev/null); then
            basename "$f"
        else
            echo "mill-board-backup: $(basename "$f") does not match its checksum: ignoring it" >&2
        fi
    done | sort
}

snapshot() {
    [[ -f $db ]] || die "no database at $db"
    mkdir -p "$dir"
    # Leftovers of a failed run: partials, and a gzip whose checksum never landed.
    rm -f -- "$dir"/.mill-board-*.partial* "$dir"/.mill-board-*.sha256.tmp
    local f
    for f in "$dir"/mill-board-*.sqlite3.gz; do
        if [[ -f $f && ! -f $f.sha256 ]]; then rm -f -- "$f"; fi
    done
    local name partial
    name="mill-board-$(date -u +%Y%m%dT%H%M%SZ).sqlite3"
    partial="$dir/.$name.partial"
    if ! copy_and_check "$db" "$partial"; then
        rm -f -- "$partial"*
        die "could not write a verified copy of $db"
    fi
    # The checksum is written under its temporary name first; the gzip takes its final name, then
    # the checksum: until that last rename the gzip is not a pair and nothing counts it.
    local sum
    if ! gzip -n -- "$partial" || ! sum=$(sha256sum <"$partial.gz"); then
        rm -f -- "$partial"*
        die "could not write the copy or its checksum"
    fi
    printf '%s  %s\n' "${sum%% *}" "$name.gz" >"$dir/.$name.sha256.tmp"
    mv "$partial.gz" "$dir/$name.gz"
    mv "$dir/.$name.sha256.tmp" "$dir/$name.gz.sha256"
    # Keep the newest N complete pairs; names sort by time.
    local old
    while IFS= read -r old; do
        rm -f -- "$dir/$old.sha256" "$dir/$old"
    done < <(pairs | head -n "-$keep")
    echo "mill-board-backup: wrote $dir/$name.gz ($(du -h "$dir/$name.gz" | cut -f1)), integrity ok"
}

# How many copies a listing holds (each is counted once, by its checksum).
copies_in() {
    grep -c 'mill-board-.*\.sqlite3\.gz\.sha256$' || true
}

ship() {
    local base=${MILL_BOARD_BACKUP_DEST:?set MILL_BOARD_BACKUP_DEST: an rsync target off this host}
    # A mounted destination that is not mounted would fill the local disk under the mount point.
    local mnt=${MILL_BOARD_BACKUP_REQUIRE_MOUNT:-}
    [[ -z $mnt ]] || mountpoint -q -- "$mnt" || die "$mnt is not mounted: not shipping"
    # One subdirectory per host, so hosts sharing a destination never prune each other.
    local dest here there stage names listing n
    dest="$base/$(hostname -s)"
    names=$(pairs)
    here=$(copies_in <<<"$(sed 's/$/.sha256/' <<<"$names")")
    [[ -n $names ]] || die "no complete copy in $dir to ship"
    # Stage links to the complete pairs only, so a stray gzip is never sent.
    stage="$dir/.ship"
    rm -rf -- "$stage"
    mkdir -p "$stage"
    while IFS= read -r n; do
        ln -s -- "$dir/$n" "$stage/$n"
        ln -s -- "$dir/$n.sha256" "$stage/$n.sha256"
    done <<<"$names"
    trap 'rm -rf -- "$stage"' RETURN
    # Pass 1 only adds (checksum last, rsync renames a file into place when it is whole).
    local filter=(--include='mill-board-*' --exclude='*')
    rsync -aL --timeout=600 "${filter[@]}" "$stage/" "$dest/" || die "shipping to $dest failed: nothing pruned"
    # Prune only once the receiver's bytes match the verified local pairs (a checksum comparison,
    # not names), and unless this host holds fewer copies than it should and fewer than the
    # destination (a wiped local directory). Otherwise the destination stays add-only.
    local differs
    differs=$(rsync -rLn --checksum --itemize-changes --timeout=600 "${filter[@]}" "$stage/" "$dest/") ||
        die "cannot verify $dest: nothing pruned"
    [[ -z $differs ]] || die "destination copies differ from the verified local ones: nothing pruned"
    listing=$(rsync --list-only --timeout=60 "$dest/" 2>/dev/null) || die "cannot list $dest: nothing pruned"
    there=$(copies_in <<<"$listing")
    if ((here < keep && here < there)); then
        echo "mill-board-backup: only $here copies here, $there at the destination; not pruning it" >&2
    else
        # The staging directory must still hold exactly the verified pairs: never prune from less.
        [[ $(find "$stage" -maxdepth 1 -type l | wc -l) -eq $((2 * $(grep -c . <<<"$names"))) ]] ||
            die "staging changed under this run: nothing pruned"
        rsync -aL --delete --timeout=600 "${filter[@]}" "$stage/" "$dest/"
    fi
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

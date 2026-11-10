#!/usr/bin/env bash
# rusty_tick backup: a snapshot of the stopped data directory, copied off the host.
#
#   backup.sh snapshot   archive RUSTY_TICK_DATA_DIR into RUSTY_TICK_BACKUP_DIR, keep the newest N
#   backup.sh ship       mirror those archives to RUSTY_TICK_BACKUP_DEST/<host> (rsync target)
#   backup.sh            both
#
# `snapshot` refuses a store that a server still holds: rusty-tick-backup.service stops
# rusty-tick around it and starts them again before `ship`. Settings come
# from tick.env (see tick.env.example); nothing secret is printed.
set -euo pipefail
shopt -s inherit_errexit
trap 'echo "rusty-tick-backup: failed at line $LINENO" >&2' ERR

die() {
    echo "rusty-tick-backup: $*" >&2
    exit 1
}

data=${RUSTY_TICK_DATA_DIR:?set RUSTY_TICK_DATA_DIR}
dir=${RUSTY_TICK_BACKUP_DIR:?set RUSTY_TICK_BACKUP_DIR}
keep=${RUSTY_TICK_BACKUP_KEEP:-14}
[[ $keep =~ ^[1-9][0-9]*$ ]] || die "RUSTY_TICK_BACKUP_KEEP must be a positive whole number, not '$keep'"
umask 077

# A copy of a live store can catch a write halfway. A multi-user server locks a user's store
# only once it opens that user, so a free lock does not prove the server is down: refuse while
# the configured address answers, then hold every lock in the data directory until the archive is
# written, so neither a server nor `rusty_tick user` can start meanwhile.
hold_locks() {
    local addr=${RUSTY_TICK_ADDR:-}
    if [[ -n $addr ]] && curl -fsS --max-time 2 "http://$addr/health" >/dev/null 2>&1; then
        die "rusty_tick answers on $addr: stop it before a snapshot"
    fi
    local lock fd
    while IFS= read -r -d '' lock; do
        exec {fd}<"$lock"
        flock -n "$fd" || die "$lock is held: stop rusty-tick before a snapshot"
    done < <(find "$data" -name '*.lock' -print0)
}

snapshot() {
    [[ -d $data ]] || die "no data directory at $data"
    hold_locks
    mkdir -p "$dir"
    rm -f -- "$dir"/.rusty-tick-*.partial
    local name partial
    name="rusty-tick-$(date -u +%Y%m%dT%H%M%SZ).tar.gz"
    partial="$dir/.$name.partial"
    if ! tar -czf "$partial" -C "$(dirname "$data")" "$(basename "$data")" ||
        ! tar -tzf "$partial" >/dev/null; then
        rm -f -- "$partial"
        die "could not write a readable archive of $data"
    fi
    mv "$partial" "$dir/$name"
    (cd "$dir" && sha256sum "$name" >"$name.sha256")
    # Keep the newest N; names sort by time.
    local old
    while IFS= read -r old; do
        rm -f -- "$dir/$old" "$dir/$old.sha256"
    done < <(cd "$dir" && printf '%s\n' rusty-tick-*.tar.gz | sort | head -n "-$keep")
    echo "rusty-tick-backup: wrote $dir/$name ($(du -h "$dir/$name" | cut -f1))"
}

# How many archive names a listing holds.
archives_in() {
    grep -c 'rusty-tick-.*\.tar\.gz$' || true
}

ship() {
    local base=${RUSTY_TICK_BACKUP_DEST:?set RUSTY_TICK_BACKUP_DEST: an rsync target off this host}
    # One subdirectory per host, so hosts sharing a destination never prune each other.
    local dest here there
    dest="$base/$(hostname -s)"
    here=$({ compgen -G "$dir/rusty-tick-*.tar.gz" || true; } | archives_in)
    ((here > 0)) || die "no archive in $dir to ship"
    there=$({ rsync --list-only --timeout=60 "$dest/" 2>/dev/null || true; } | archives_in)
    # Mirror the newest N, unless this host now has fewer than it should and fewer than the
    # destination (a wiped local directory): then add without pruning, and say so.
    local prune=(--delete)
    if ((here < keep && here < there)); then
        prune=()
        echo "rusty-tick-backup: only $here archives here, $there at the destination; not pruning it" >&2
    fi
    rsync -a "${prune[@]}" --timeout=600 --include='rusty-tick-*' --exclude='*' "$dir/" "$dest/"
    echo "rusty-tick-backup: shipped to $dest"
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

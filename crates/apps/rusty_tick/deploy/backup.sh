#!/usr/bin/env bash
# rusty_tick backup: a snapshot of the stopped data directory, copied off the host.
#
#   backup.sh snapshot   archive RUSTY_TICK_DATA_DIR into RUSTY_TICK_BACKUP_DIR, keep the newest N
#   backup.sh ship       mirror those archives to RUSTY_TICK_BACKUP_DEST (rsync target)
#   backup.sh            both
#
# `snapshot` refuses a store that a server still holds: rusty-tick-backup.service stops
# rusty-tick around it and starts them again before `ship`. Settings come
# from tick.env (see tick.env.example); nothing secret is printed.
set -euo pipefail
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

# Fail when any store under the data directory is still locked by a running server: a copy of a
# live store can catch a write halfway.
check_stopped() {
    local lock
    while IFS= read -r -d '' lock; do
        flock -n "$lock" true || die "$lock is held: stop rusty-tick before a snapshot"
    done < <(find "$data" -name store.lock -print0)
}

snapshot() {
    [[ -d $data ]] || die "no data directory at $data"
    check_stopped
    mkdir -p "$dir"
    local name partial
    name="rusty-tick-$(date -u +%Y%m%dT%H%M%SZ).tar.gz"
    partial="$dir/.$name.partial"
    tar -czf "$partial" -C "$(dirname "$data")" "$(basename "$data")"
    tar -tzf "$partial" >/dev/null || die "the new archive does not read back"
    mv "$partial" "$dir/$name"
    (cd "$dir" && sha256sum "$name" >"$name.sha256")
    # Keep the newest N; names sort by time.
    local old
    while IFS= read -r old; do
        rm -f -- "$dir/$old" "$dir/$old.sha256"
    done < <(cd "$dir" && printf '%s\n' rusty-tick-*.tar.gz | sort | head -n "-$keep")
    echo "rusty-tick-backup: wrote $dir/$name ($(du -h "$dir/$name" | cut -f1))"
}

ship() {
    local dest=${RUSTY_TICK_BACKUP_DEST:?set RUSTY_TICK_BACKUP_DEST: an rsync target off this host}
    compgen -G "$dir/rusty-tick-*.tar.gz" >/dev/null || die "no archive in $dir to ship"
    # A mirror of the archives only: the destination keeps the same newest N.
    rsync -a --delete --include='rusty-tick-*' --exclude='*' "$dir/" "$dest/"
    echo "rusty-tick-backup: shipped to $dest"
}

case ${1:-all} in
snapshot) snapshot ;;
ship) ship ;;
all) snapshot && ship ;;
*) die "usage: backup.sh [snapshot|ship]" ;;
esac

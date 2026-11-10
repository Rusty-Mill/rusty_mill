#!/usr/bin/env bash
# rusty_tick restore drill: prove a backup opens and holds tasks, without touching the live store.
#
#   restore-drill.sh [ARCHIVE]   default: the newest archive in RUSTY_TICK_BACKUP_DIR
#
# Checks the archive's sha256, unpacks it into a temporary directory, serves it with a
# throwaway rusty_tick on a spare loopback port, and counts each user's tasks through the API.
# Tokens it needs are minted in the temporary copy only and deleted with it. Fails if the
# archive is damaged, the server does not come up, or no task is found.
set -euo pipefail
trap 'echo "restore-drill: failed at line $LINENO" >&2' ERR

die() {
    echo "restore-drill: $*" >&2
    exit 1
}

bin=${RUSTY_TICK_BIN:-rusty_tick}
port=${RUSTY_TICK_DRILL_PORT:-18788}
archive=${1:-}
if [[ -z $archive ]]; then
    dir=${RUSTY_TICK_BACKUP_DIR:?set RUSTY_TICK_BACKUP_DIR or name an archive}
    archive=$(compgen -G "$dir/rusty-tick-*.tar.gz" | sort | tail -n 1 || true)
    [[ -n $archive ]] || die "no archive in $dir"
fi
[[ -f $archive ]] || die "no archive at $archive"
if [[ -f $archive.sha256 ]]; then
    (cd "$(dirname "$archive")" && sha256sum --quiet -c "$(basename "$archive").sha256") ||
        die "$archive does not match its sha256"
else
    echo "restore-drill: no $archive.sha256; checking the archive reads back only" >&2
fi

scratch=$(mktemp -d)
server=
cleanup() {
    local status=$?
    if [[ -n $server ]]; then
        kill "$server" 2>/dev/null || true
        wait "$server" 2>/dev/null || true
    fi
    rm -rf -- "$scratch"
    exit "$status"
}
trap cleanup EXIT
umask 077
tar -xzf "$archive" -C "$scratch" || die "$archive does not unpack"
top=$(tar -tzf "$archive" | awk -F/ 'NR == 1 { print $1 }')
data=$scratch/$top
[[ -d $data ]] || die "$archive holds no data directory"

# One drill token per enabled user (multi-user), or a throwaway shared token (single-user).
declare -A tokens
if [[ -f $data/users.json ]]; then
    while IFS= read -r user; do
        tokens[$user]=$("$bin" user add "$user" drill --data-dir "$data")
    done < <("$bin" user list --data-dir "$data" | grep -v '^ ' | grep -v ' (disabled)$')
    unset RUSTY_TICK_TOKEN
else
    tokens[single]=drill-$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
    export RUSTY_TICK_TOKEN=${tokens[single]}
fi
(( ${#tokens[@]} > 0 )) || die "the backup has no enabled user"

unset RUSTY_TICK_ISSUER
"$bin" --data-dir "$data" --addr "127.0.0.1:$port" >"$scratch/server.log" 2>&1 &
server=$!
base=http://127.0.0.1:$port
for _ in $(seq 50); do
    curl -fsS "$base/health" >/dev/null 2>&1 && break
    kill -0 "$server" 2>/dev/null || die "rusty_tick did not start: $(tail -n 5 "$scratch/server.log")"
    sleep 0.2
done
curl -fsS "$base/health" >/dev/null || die "rusty_tick did not answer /health on port $port"

# Tasks of every list in every status, counted by their one listId field each.
count() {
    local auth=(-H "Authorization: Bearer $1") n=0 list status
    for list in $(curl -fsS "${auth[@]}" "$base/api/v1/lists" | grep -o '"id":"[0-9a-f-]*"' | cut -d'"' -f4); do
        for status in open "done" wontdo; do
            n=$((n + $(curl -fsS "${auth[@]}" "$base/api/v1/lists/$list/tasks?status=$status" |
                grep -o '"listId"' | wc -l)))
        done
    done
    echo "$n"
}

total=0
for user in "${!tokens[@]}"; do
    n=$(count "${tokens[$user]}")
    echo "restore-drill: $user has $n tasks"
    total=$((total + n))
done
(( total > 0 )) || die "the backup opened but holds no tasks"
echo "restore-drill: ok, $(basename "$archive") restores $total tasks"

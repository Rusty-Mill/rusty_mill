#!/usr/bin/env bash
# rusty_tick restore drill: prove a backup opens and holds tasks, without touching the live store.
#
#   restore-drill.sh [ARCHIVE]   default: the newest archive in RUSTY_TICK_BACKUP_DIR
#
# Checks the archive's sha256, unpacks it into a temporary directory, serves it with a
# throwaway rusty_tick on a spare loopback port, and counts each user's tasks through the API.
# Tokens it needs are minted in the temporary copy only and deleted with it. Fails if the
# archive is damaged, the server does not come up, any enabled user's tasks cannot be read, or
# no task is found.
set -euo pipefail
shopt -s inherit_errexit
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

# One drill token per enabled user (multi-user), or a throwaway shared token (single-user). Each
# is kept as a header file in the temporary copy, so no token reaches a command line.
declare -A auth=()
users=0
if [[ -f $data/users.json ]]; then
    while IFS= read -r user; do
        users=$((users + 1))
        auth[$user]=$scratch/auth-$users
        printf 'Authorization: Bearer %s\n' "$("$bin" user add "$user" drill --data-dir "$data")" \
            >"${auth[$user]}"
    done < <("$bin" user list --data-dir "$data" | grep -v '^ ' | grep -v ' (disabled)$' || true)
    unset RUSTY_TICK_TOKEN
else
    token=drill-$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
    auth[single]=$scratch/auth-0
    printf 'Authorization: Bearer %s\n' "$token" >"${auth[single]}"
    export RUSTY_TICK_TOKEN=$token
fi
((${#auth[@]} > 0)) || die "the backup has no enabled user"

unset RUSTY_TICK_ISSUER
base=http://127.0.0.1:$port
if curl -fsS --max-time 2 "$base/health" >/dev/null 2>&1; then
    die "port $port is already in use; set RUSTY_TICK_DRILL_PORT to a free one"
fi
"$bin" --data-dir "$data" --addr "127.0.0.1:$port" >"$scratch/server.log" 2>&1 &
server=$!
for _ in $(seq 50); do
    curl -fsS "$base/health" >/dev/null 2>&1 && break
    kill -0 "$server" 2>/dev/null || die "rusty_tick did not start: $(tail -n 5 "$scratch/server.log")"
    sleep 0.2
done
curl -fsS "$base/health" >/dev/null || die "rusty_tick did not answer /health on port $port"

# How many times a pattern occurs in some text (0 when none).
occurrences() {
    { grep -o "$1" <<<"$2" || true; } | wc -l
}

# A user's tasks in every list and status, counted by their one listId field each. Fails if
# any request fails.
count() {
    local header=$1 lists list status page n=0
    lists=$(curl -fsS -H "@$header" "$base/api/v1/lists") || return 1
    for list in $({ grep -o '"id":"[0-9a-f-]*"' <<<"$lists" || true; } | cut -d'"' -f4); do
        for status in open "done" wontdo; do
            page=$(curl -fsS -H "@$header" "$base/api/v1/lists/$list/tasks?status=$status") || return 1
            n=$((n + $(occurrences '"listId"' "$page")))
        done
    done
    echo "$n"
}

total=0
for user in "${!auth[@]}"; do
    n=$(count "${auth[$user]}") || die "could not read $user's tasks from the restored copy"
    echo "restore-drill: $user has $n tasks"
    total=$((total + n))
done
((total > 0)) || die "the backup opened but holds no tasks"
echo "restore-drill: ok, $(basename "$archive") restores $total tasks"

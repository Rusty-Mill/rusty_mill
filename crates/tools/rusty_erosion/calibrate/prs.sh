#!/bin/sh
# prs.sh: every merged non-bot PR of Rusty-Mill/rusty_mill as "NUMBER<TAB>BRANCH<TAB>MERGE_SHA",
# for measure.sh (card #704 slice 5, D110). Needs `gh`. Pages by hand: `--paginate` follows
# numeric-id links some proxies refuse.
set -eu
p=1
while :; do
  page=$(gh api "repos/Rusty-Mill/rusty_mill/pulls?state=closed&per_page=100&page=$p")
  printf '%s' "$page" | jq -r '.[] | select(.merged_at != null and .user.type != "Bot") | [.number, .head.ref, .merge_commit_sha] | @tsv'
  [ "$(printf '%s' "$page" | jq length)" -lt 100 ] && break
  p=$((p + 1))
done

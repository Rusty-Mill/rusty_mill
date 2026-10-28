#!/usr/bin/env bash
# Tag HEAD as the next libs-YYYY.MM.N and push that one tag to origin (ADR-0013).
#
# N counts from 0 within the UTC month (LIBS_TAG_MONTH overrides the month, for
# tests). A HEAD that already carries a libs-* tag is left alone. The push never
# forces, so a tag that appeared on origin meanwhile rejects it and stays as it was.
set -euo pipefail

month="${LIBS_TAG_MONTH:-$(date -u +%Y.%m)}"
git fetch --quiet origin 'refs/tags/libs-*:refs/tags/libs-*'

head="$(git rev-parse HEAD)"
existing="$(git tag --points-at "$head" --list 'libs-*' | head -n 1)"
if [ -n "$existing" ]; then
  echo "HEAD ($head) already has $existing"
  exit 0
fi

# Only decimal suffixes count, read in base 10 so a stray "08" is not octal.
last="$(git tag --list "libs-${month}.*" \
  | sed -n "s/^libs-${month//./\\.}\.\([0-9][0-9]*\)$/\1/p" | sort -n | tail -n 1)"
if [ -z "$last" ]; then
  n=0
else
  n=$((10#$last + 1))
fi
tag="libs-${month}.${n}"

git tag "$tag" "$head"
git push origin "refs/tags/$tag"
echo "Tagged $head as $tag"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  echo "Tagged $head as $tag" >> "$GITHUB_STEP_SUMMARY"
fi

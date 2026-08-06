#!/usr/bin/env bash
#
# Every `tag = "vX.Y.Z"` in the docs and the template must name the current
# crate version.
#
# This exists because that exact rot has already happened here: the README told
# readers to depend on `tag = "v0.2.0"` with `features = ["otel"]`, a tag that
# predated the feature, so following the instructions verbatim failed to
# compile. Nothing caught it, because nothing was looking.
#
# Historical release links in the CHANGELOG use a different form
# (`.../releases/tag/v0.1.0`) and are deliberately not matched — those are
# supposed to name old versions.
#
# The match is textual, so prose *quoting* an old snippet trips it too. That is
# the safe direction to be wrong in: reword the sentence rather than teaching
# this script to guess which occurrences are instructions.
#
#     scripts/check-versions.sh

set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' crates/rusty-mcp/Cargo.toml | head -n1)"
if [ -z "$version" ]; then
    echo "error: could not read the version from crates/rusty-mcp/Cargo.toml" >&2
    exit 1
fi
expected="v$version"

echo "==> rusty-mcp is $version, so install snippets must say $expected"

status=0
found_any=0

while IFS= read -r match; do
    [ -z "$match" ] && continue
    found_any=1

    file="${match%%:*}"
    rest="${match#*:}"
    line="${rest%%:*}"
    tag="$(printf '%s' "$match" | sed 's/.*tag = "\([^"]*\)".*/\1/')"

    if [ "$tag" != "$expected" ]; then
        echo "$file:$line: says $tag, expected $expected" >&2
        status=1
    fi
done <<<"$(grep -rn 'tag = "v[0-9][^"]*"' README.md CHANGELOG.md template/ || true)"

if [ "$found_any" -eq 0 ]; then
    # Not a pass. If the install snippets vanished, this check is silently
    # guarding nothing, which is worse than failing.
    echo "error: no version-tagged install snippets found at all" >&2
    exit 1
fi

if [ "$status" -ne 0 ]; then
    echo >&2
    echo "Bump these alongside the crate version, or the instructions point at" >&2
    echo "a release that does not contain the code they describe." >&2
    exit 1
fi

echo "==> every install snippet names $expected"

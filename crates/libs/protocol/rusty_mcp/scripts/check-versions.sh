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
# # Exempting prose
#
# The match is textual, so a sentence *quoting* an old snippet trips it too.
# That happened three times, every time in a passage explaining the very bug
# this script exists to prevent, and each time the fix was to reword the
# documentation into something vaguer. A check that makes the docs worse every
# time it fires is taxing the wrong thing.
#
# So an occurrence can be exempted with a marker:
#
#     <!-- check-versions: ignore -->
#
# It covers the rest of its own line, the line after it, and — when that next
# line opens a fenced code block — the whole block. That last rule exists
# because the most useful thing to write is the snippet that actually misled
# someone, and a marker cannot go inside a fence without rendering as code.
#
# Nothing else is inferred: no guessing at blockquotes, no ranges. Writing the
# marker states that you know this names an old version on purpose. List every
# exemption with:
#
#     scripts/check-versions.sh --list-exemptions
#
# Usage:
#
#     scripts/check-versions.sh                    # check
#     scripts/check-versions.sh --list-exemptions  # show what is exempted
#     scripts/check-versions.sh --self-test        # prove both directions work

set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

MARKER='check-versions: ignore'
PATTERN='tag = "v[0-9][^"]*"'

# Files and directories carrying install snippets. Deliberately not the whole
# repo: the check should fail loudly if one of these stops existing.
targets() {
    printf '%s\n' README.md CHANGELOG.md
    find template -type f \( -name '*.toml' -o -name '*.md' \) 2>/dev/null || true
}

# Print `file:line:{exempt|check}:text` for every occurrence.
scan() {
    local file line_no line armed in_fence exempt
    while IFS= read -r file; do
        [ -f "$file" ] || continue
        line_no=0
        armed=0
        in_fence=0

        while IFS= read -r line || [ -n "$line" ]; do
            line_no=$((line_no + 1))
            exempt=0

            if [ "$in_fence" -eq 1 ]; then
                exempt=1
                # The closing fence ends the exemption.
                [[ "$line" == '```'* ]] && in_fence=0
            elif [ "$armed" -eq 1 ]; then
                exempt=1
                armed=0
                # An opening fence right after the marker extends it to the
                # whole block.
                [[ "$line" == '```'* ]] && in_fence=1
            elif [[ "$line" == *"$MARKER"* ]]; then
                exempt=1
                armed=1
            fi

            if [[ "$line" =~ $PATTERN ]]; then
                if [ "$exempt" -eq 1 ]; then
                    printf '%s:%s:exempt:%s\n' "$file" "$line_no" "$line"
                else
                    printf '%s:%s:check:%s\n' "$file" "$line_no" "$line"
                fi
            fi
        done <"$file"
    done < <(targets)
}

cd "$repo"

case "${1:-}" in
--list-exemptions)
    scan | grep ':exempt:' || echo "(none)"
    exit 0
    ;;
--self-test)
    # Both directions, on throwaway fixtures — a check nobody has watched fail
    # is a check nobody knows works.
    work="$(mktemp -d)"
    trap 'rm -rf "$work"' EXIT
    cd "$work"

    # The script derives the repo root from its own location, so the copy has
    # to sit in a `scripts/` directory exactly as it does here.
    mkdir -p template crates/rusty-mcp scripts
    printf 'version = "9.9.9"\n' >crates/rusty-mcp/Cargo.toml
    printf 'rusty-mcp = { git = "...", tag = "v9.9.9" }\n' >CHANGELOG.md
    cp "$repo/scripts/check-versions.sh" scripts/check.sh

    printf 'a stale one: tag = "v0.0.1"\n' >README.md
    if scripts/check.sh >/dev/null 2>&1; then
        echo "self-test FAILED: an unmarked stale tag was accepted" >&2
        exit 1
    fi

    printf '<!-- check-versions: ignore -->\na stale one: tag = "v0.0.1"\n' >README.md
    if ! scripts/check.sh >/dev/null 2>&1; then
        echo "self-test FAILED: a marked stale tag was still rejected" >&2
        exit 1
    fi

    printf 'a stale one: tag = "v0.0.1" <!-- check-versions: ignore -->\n' >README.md
    if ! scripts/check.sh >/dev/null 2>&1; then
        echo "self-test FAILED: a same-line marker was not honoured" >&2
        exit 1
    fi

    printf '<!-- check-versions: ignore -->\n```toml\ntag = "v0.0.1"\n```\n' >README.md
    if ! scripts/check.sh >/dev/null 2>&1; then
        echo "self-test FAILED: a marked fenced block was not exempted" >&2
        exit 1
    fi

    # The exemption must end with the fence, not run to end of file.
    printf '<!-- check-versions: ignore -->\n```toml\nx = 1\n```\ntag = "v0.0.1"\n' >README.md
    if scripts/check.sh >/dev/null 2>&1; then
        echo "self-test FAILED: the exemption leaked past the closing fence" >&2
        exit 1
    fi

    # Everything exempted must not read as a pass: the check would be guarding
    # nothing.
    printf '<!-- check-versions: ignore -->\nrusty-mcp = { tag = "v9.9.9" }\n' >CHANGELOG.md
    printf '<!-- check-versions: ignore -->\ntag = "v0.0.1"\n' >README.md
    if scripts/check.sh >/dev/null 2>&1; then
        echo "self-test FAILED: exempting everything was treated as a pass" >&2
        exit 1
    fi

    echo "==> self-test passed"
    exit 0
    ;;
"") ;;
*)
    echo "unknown argument: $1" >&2
    exit 2
    ;;
esac

version="$(sed -n 's/^version = "\(.*\)"/\1/p' crates/rusty-mcp/Cargo.toml | head -n1)"
if [ -z "$version" ]; then
    echo "error: could not read the version from crates/rusty-mcp/Cargo.toml" >&2
    exit 1
fi
expected="v$version"

echo "==> rusty-mcp is $version, so install snippets must say $expected"

status=0
checked=0
exempt=0

while IFS= read -r entry; do
    [ -z "$entry" ] && continue

    file="${entry%%:*}"
    rest="${entry#*:}"
    line="${rest%%:*}"
    rest="${rest#*:}"
    kind="${rest%%:*}"
    text="${rest#*:}"

    if [ "$kind" = exempt ]; then
        exempt=$((exempt + 1))
        continue
    fi

    checked=$((checked + 1))
    tag="$(printf '%s' "$text" | sed 's/.*tag = "\([^"]*\)".*/\1/')"
    if [ "$tag" != "$expected" ]; then
        echo "$file:$line: says $tag, expected $expected" >&2
        status=1
    fi
done <<<"$(scan)"

if [ "$checked" -eq 0 ]; then
    # Not a pass. If every snippet vanished or was exempted, this check is
    # silently guarding nothing, which is worse than failing.
    echo "error: no version-tagged install snippets left to check" >&2
    echo "       ($exempt exempted — see --list-exemptions)" >&2
    exit 1
fi

if [ "$status" -ne 0 ]; then
    echo >&2
    echo "Bump these alongside the crate version, or the instructions point at" >&2
    echo "a release that does not contain the code they describe." >&2
    echo "If an occurrence names an old version on purpose, mark it:" >&2
    echo "    <!-- $MARKER -->" >&2
    exit 1
fi

echo "==> $checked install snippet(s) name $expected, $exempt exempted"

#!/usr/bin/env bash
#
# Verify a published tag is actually consumable.
#
# `check-template.sh` rewrites the generated project's dependency to a **path**
# so a library change that breaks the template fails the pull request that made
# it. That is the right trade for pre-merge CI, but it means the **git**
# dependency — the way every real consumer takes this crate — is never
# exercised. This script covers exactly that gap, and can only run once a tag
# exists.
#
# It is not hypothetical. v0.4.0 shipped `template/Cargo.toml` holding
# `name = "{{project-name}}"`, and because cargo scans a git checkout for
# manifests when resolving a git dependency, every consumer saw
#
#     error: invalid character `{` in package name: `{{project-name}}`
#
# on every build. CI was green throughout, because CI never resolved a git
# dependency. It was found by hand, after tagging.
#
# # The build succeeded
#
# That is the part worth designing around. The failure was a **zero exit code
# with an error line in the output**, so a check that only looks at exit status
# would have passed it. This one scans what cargo actually printed.
#
# Usage:
#
#     scripts/check-published-tag.sh v0.5.0
#     scripts/check-published-tag.sh            # uses $GITHUB_REF_NAME in CI
#     scripts/check-published-tag.sh --self-test

set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Lines that mean the tag is not cleanly consumable, even on a zero exit.
FATAL_PATTERNS='^error|invalid character'

# Worth surfacing but not worth failing a release over: a warning from a
# third-party dependency is not something this repo can fix.
NOTICE_PATTERNS='^warning'

# Print any fatal lines in $1. Returns 1 if there were any.
scan_output() {
    local log="$1" hits
    hits="$(grep -nE "$FATAL_PATTERNS" "$log" || true)"
    if [ -n "$hits" ]; then
        echo "$hits"
        return 1
    fi
    return 0
}

if [ "${1:-}" = --self-test ]; then
    # The detector is the novel part, so it gets proven rather than assumed.
    work="$(mktemp -d)"
    trap 'rm -rf "$work"' EXIT

    printf '   Compiling foo v0.1.0\n    Finished dev profile\n' >"$work/clean.log"
    scan_output "$work/clean.log" >/dev/null || {
        echo "self-test FAILED: clean output was rejected" >&2
        exit 1
    }

    # The exact 0.4.0 line, which accompanied a *successful* build.
    printf '   Compiling foo v0.1.0\nerror: invalid character `{` in package name: `{{project-name}}`\n    Finished dev profile\n' \
        >"$work/dirty.log"
    if scan_output "$work/dirty.log" >/dev/null; then
        echo "self-test FAILED: the v0.4.0 error line was not detected" >&2
        exit 1
    fi

    echo "==> self-test passed"
    exit 0
fi

tag="${1:-${GITHUB_REF_NAME:-}}"
if [ -z "$tag" ]; then
    echo "usage: scripts/check-published-tag.sh <tag>" >&2
    exit 2
fi

if ! command -v cargo-generate >/dev/null 2>&1; then
    echo "error: cargo-generate is not installed" >&2
    echo "       cargo install cargo-generate --locked" >&2
    exit 2
fi

url="$(sed -n 's/^repository = "\(.*\)"/\1/p' "$repo/Cargo.toml" | head -n1)"
if [ -z "$url" ]; then
    echo "error: could not read the repository URL from Cargo.toml" >&2
    exit 1
fi

echo "==> verifying $tag is consumable from $url"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cd "$work"

# The real command from the README, pinned to the tag under test: the template
# as it shipped *in that release*, not as it looks on the default branch.
echo "==> cargo generate --git $url --tag $tag template"
cargo generate --git "$url" --tag "$tag" template \
    --name release-check \
    --define description="Verifying the published tag" \
    --silent

cd release-check

# The dependency is left exactly as the template ships it. Rewriting it here
# would recreate the blind spot this script exists to close.
dep="$(grep 'rusty-mcp = ' Cargo.toml || true)"
echo "==> dependency: $dep"

case "$dep" in
*"tag = \"$tag\""*) ;;
*)
    echo "error: the generated project does not depend on $tag" >&2
    echo "       the template's tag reference was not bumped for this release" >&2
    exit 1
    ;;
esac

export CARGO_TARGET_DIR="$work/target"
status=0

for step in build test; do
    echo "==> cargo $step"
    if ! cargo "$step" >"$work/$step.log" 2>&1; then
        echo "error: cargo $step failed" >&2
        tail -30 "$work/$step.log" >&2
        exit 1
    fi

    # The build succeeded. That is not the same as being clean — see the header.
    if ! scan_output "$work/$step.log"; then
        echo "error: cargo $step succeeded but printed the above" >&2
        status=1
    fi

    notices="$(grep -nE "$NOTICE_PATTERNS" "$work/$step.log" || true)"
    if [ -n "$notices" ]; then
        echo "note: cargo $step also printed warnings:"
        echo "$notices"
    fi
done

if [ "$status" -ne 0 ]; then
    echo >&2
    echo "The tag builds, but a consumer sees that output on every build of" >&2
    echo "their own project. Fix it and publish a patch release." >&2
    exit 1
fi

echo "==> $tag is cleanly consumable"

#!/usr/bin/env bash
#
# Instantiate `template/` and build and test the result.
#
# A template is a second copy of the API surface, and second copies rot. This
# is the thing that makes the copy honest: a change to the library that breaks
# the template fails the pull request that made it, rather than the next person
# who runs `cargo generate`.
#
# The generated project's dependency is rewritten from the published tag to a
# path pointing at this checkout. That is deliberate — the point is to catch a
# template that no longer compiles against the code *in this commit*, which a
# tagged dependency would hide until the next release.
#
#     scripts/check-template.sh

set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
name="template-check"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

project="$work/$name"
cp -R "$repo/template" "$project"

# `cargo generate` is not installed in CI, so substitute the placeholders the
# same way it would. Keep this list in step with `template/cargo-generate.toml`.
find "$project" -type f \( -name '*.rs' -o -name '*.toml' -o -name '*.md' \) -print0 |
    while IFS= read -r -d '' file; do
        sed -i \
            -e "s/{{project-name}}/$name/g" \
            -e "s/{{description}}/A generated MCP server/g" \
            "$file"
    done

rm -f "$project/cargo-generate.toml"

if grep -rn '{{' "$project" --include='*.rs' --include='*.toml' --include='*.md'; then
    echo "error: a placeholder survived substitution — add it to this script" >&2
    exit 1
fi

# Against this checkout, not the tag. See the note above.
sed -i \
    -e "s|rusty-mcp = { git = \"[^\"]*\", tag = \"[^\"]*\" }|rusty-mcp = { path = \"$repo/crates/rusty-mcp\" }|" \
    "$project/Cargo.toml"

if ! grep -q 'rusty-mcp = { path =' "$project/Cargo.toml"; then
    echo "error: the rusty-mcp dependency was not rewritten to a path" >&2
    echo "       the git+tag line in template/Cargo.toml must have changed shape" >&2
    exit 1
fi

echo "==> building the generated project in $project"
cd "$project"

# Its own target directory: sharing the workspace's would let a stale artifact
# hide a break.
export CARGO_TARGET_DIR="$work/target"

cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets

echo "==> the template still builds"

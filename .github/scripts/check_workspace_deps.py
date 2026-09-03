#!/usr/bin/env python3
"""Fail when a workspace member is resolved from git/registry instead of a
local path dependency.

ATLAS-RWC-0050 requires that a first-party crate present in the same
workspace resolve locally rather than through a remote source. Cargo does
not enforce this itself: nothing stops a manifest from pinning
`{ git = "...", rev = "..." }` for a crate that also lives under `crates/`
in this same workspace, and the two copies can silently diverge (this
happened for `rusty_lsp` and `rusty_simd` -- see ADR-0002).

Reads `cargo metadata --format-version=1 --all-features` JSON from the path
given as argv[1] and prints one violation per matching non-workspace
package resolved from a git source whose name collides with a workspace
member's name.
"""
import json
import sys
from pathlib import Path


def git_shadowed_workspace_members(metadata: dict) -> list[str]:
    """Return sorted descriptions of packages that shadow a workspace member.

    A violation is a resolved package that is *not* itself a workspace
    member, whose `source` is a git source, and whose `name` matches a
    workspace member's name -- i.e. a git copy of a crate that also has a
    local, path-resolved copy in this same workspace.
    """
    workspace_ids = set(metadata["workspace_members"])
    workspace_names = {
        pkg["name"] for pkg in metadata["packages"] if pkg["id"] in workspace_ids
    }

    violations = []
    for pkg in metadata["packages"]:
        if pkg["id"] in workspace_ids:
            continue
        source = pkg.get("source") or ""
        if source.startswith("git+") and pkg["name"] in workspace_names:
            violations.append(f"{pkg['name']} (resolved from {source})")

    return sorted(violations)


def main() -> None:
    metadata = json.loads(Path(sys.argv[1]).read_text())
    violations = git_shadowed_workspace_members(metadata)

    if not violations:
        return

    print(
        "ATLAS-RWC-0050 violation: the following crates are both a workspace "
        "member (resolved via a path dependency) and pinned elsewhere as a "
        "git dependency. Point the consuming manifest at the workspace copy "
        "instead (a path dependency, or `crate = { workspace = true }` if it "
        "is also listed under [workspace.dependencies]):",
        file=sys.stderr,
    )
    for violation in violations:
        print(f"  - {violation}", file=sys.stderr)
    sys.exit(1)


if __name__ == "__main__":
    main()

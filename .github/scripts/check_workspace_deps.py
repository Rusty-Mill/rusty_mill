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

It also pins the direct external dependencies of the crates in
`PINNED_EXTERNAL_DEPENDENCIES` (ADR-0002, sovereignty list item 10), and allows
`tracing` and `tracing-*` as direct dependencies of one workspace crate only,
`rusty_log` (`docs/research/TRACING-FACADE-PLAN.md`).
"""
import json
import sys
from pathlib import Path


# Crates whose direct external dependencies (normal or build, any feature;
# dev-dependencies are exempt, ADR-0002) are an explicit list, each with the
# reason it stays. The list only shrinks: a dependency that is not on it is a
# violation, and so is an entry the crate no longer depends on, so finishing
# a migration means deleting its line here.
#
# `rusty-mcp` is Tier T under ADR-0002 Amendment 1: sovereign except for these.
PINNED_EXTERNAL_DEPENDENCIES: dict[str, dict[str, str]] = {
    "rusty-mcp": {
        "http": "request and response types of the tower layers; adapter boundary only (D4)",
        "tower-layer": "the Layer impls of the tower adapters, until the gateway leaves axum",
        "tower-service": "the Service impls of the tower adapters, until the gateway leaves axum",
        "tokio": "tokio::time in the tower adapter's timeout; the layers run on the gateway's runtime",
    },
}

# The facade itself is the one crate allowed to depend on `tracing` for good.
TRACING_FACADE = "rusty_log"


def _is_tracing(name: str) -> bool:
    return name == "tracing" or name.startswith("tracing-")


def tracing_facade_violations(
    metadata: dict,
    facade: str = TRACING_FACADE,
) -> list[str]:
    """Return sorted descriptions of workspace crates, other than the facade,
    that depend directly on `tracing` or a `tracing-*` crate. Any dependency
    kind counts, and path or registry source does not matter. Only workspace
    members are checked: Nexus is excluded and frozen, and third-party crates
    keep their own `tracing` dependency.
    """
    workspace_ids = set(metadata["workspace_members"])
    violations = []
    for pkg in metadata["packages"]:
        if pkg["id"] not in workspace_ids or pkg["name"] == facade:
            continue
        for name in sorted({d["name"] for d in pkg.get("dependencies", []) if _is_tracing(d["name"])}):
            violations.append(
                f"{pkg['name']}: depends on `{name}` directly; use `{facade}` instead "
                "(docs/research/TRACING-FACADE-PLAN.md)"
            )
    return sorted(violations)


def pinned_dependency_violations(
    metadata: dict,
    pinned: dict[str, dict[str, str]] = PINNED_EXTERNAL_DEPENDENCIES,
) -> list[str]:
    """Return sorted descriptions of pinned crates whose external dependencies
    differ from their allow-list.

    A dependency is external when it has a `source` (a registry or git
    package; path dependencies have none) and its kind is normal or build.
    """
    workspace_ids = set(metadata["workspace_members"])
    violations = []
    for pkg in metadata["packages"]:
        allowed = pinned.get(pkg["name"])
        if allowed is None or pkg["id"] not in workspace_ids:
            continue
        actual = {
            dep["name"]
            for dep in pkg.get("dependencies", [])
            if dep.get("source") and dep.get("kind") in (None, "build")
        }
        for name in sorted(actual - allowed.keys()):
            violations.append(
                f"{pkg['name']}: external dependency `{name}` is not on its allow-list; "
                "remove it, or add it to PINNED_EXTERNAL_DEPENDENCIES with the reason it stays"
            )
        for name in sorted(allowed.keys() - actual):
            violations.append(
                f"{pkg['name']}: `{name}` is on the allow-list but is no longer a dependency; "
                "delete its entry from PINNED_EXTERNAL_DEPENDENCIES"
            )
    return sorted(violations)


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
    failed = False

    shadowed = git_shadowed_workspace_members(metadata)
    if shadowed:
        failed = True
        print(
            "ATLAS-RWC-0050 violation: the following crates are both a workspace "
            "member (resolved via a path dependency) and pinned elsewhere as a "
            "git dependency. Point the consuming manifest at the workspace copy "
            "instead (a path dependency, or `crate = { workspace = true }` if it "
            "is also listed under [workspace.dependencies]):",
            file=sys.stderr,
        )
        for violation in shadowed:
            print(f"  - {violation}", file=sys.stderr)

    pinned = pinned_dependency_violations(metadata)
    if pinned:
        failed = True
        print(
            "Pinned external dependencies (ADR-0002, sovereignty list item 10) "
            "differ from the manifests:",
            file=sys.stderr,
        )
        for violation in pinned:
            print(f"  - {violation}", file=sys.stderr)

    facade = tracing_facade_violations(metadata)
    if facade:
        failed = True
        print(
            "Only `rusty_log` may depend on `tracing` directly "
            "(docs/research/TRACING-FACADE-PLAN.md):",
            file=sys.stderr,
        )
        for violation in facade:
            print(f"  - {violation}", file=sys.stderr)

    if failed:
        sys.exit(1)


if __name__ == "__main__":
    main()

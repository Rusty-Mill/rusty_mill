#!/usr/bin/env python3
"""Fail when a workspace crate declares a `rust-version` below what its dependencies declare.

Read `cargo metadata` JSON (default features, so optional dependencies a plain build
does not pull are not counted) from argv[1]. For every workspace member that declares
`rust-version`, take the highest `rust-version` declared by any package in its resolved
normal and build dependency closure (an edition-2024 package counts as 1.85) and report
the member if its own declaration is lower. This compares declarations; it does not
build at the old toolchain.
"""
import json
import sys
from pathlib import Path

EDITION_2024_FLOOR = (1, 85)


def _version(text: str | None) -> tuple[int, int]:
    """Parse `major.minor[.patch]` as (major, minor); no declaration is (0, 0)."""
    if not text:
        return (0, 0)
    parts = [int(p) for p in text.split(".")]
    return (parts[0], parts[1] if len(parts) > 1 else 0)


def _floor(pkg: dict) -> tuple[int, int]:
    """The toolchain floor a package declares, or implies through its edition."""
    implied = EDITION_2024_FLOOR if pkg.get("edition") == "2024" else (0, 0)
    return max(_version(pkg.get("rust_version")), implied)


def _closure(start: str, nodes: dict[str, dict]) -> set[str]:
    """Package IDs reachable from `start` through normal and build dependencies."""
    seen: set[str] = set()
    stack = [start]
    while stack:
        for dep in nodes.get(stack.pop(), {}).get("deps", []):
            kinds = {kind["kind"] for kind in dep["dep_kinds"]}
            if (None in kinds or "build" in kinds) and dep["pkg"] not in seen:
                seen.add(dep["pkg"])
                stack.append(dep["pkg"])
    return seen


def rust_version_violations(metadata: dict) -> list[str]:
    """Return sorted descriptions of members declaring less than their dependencies need."""
    packages = {pkg["id"]: pkg for pkg in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    violations = []
    for member in metadata["workspace_members"]:
        declared = packages[member].get("rust_version")
        if not declared:
            continue
        needed, via = max(
            ((_floor(packages[dep]), packages[dep]["name"]) for dep in _closure(member, nodes)),
            default=((0, 0), ""),
        )
        if needed > _version(declared):
            violations.append(
                f"{packages[member]['name']}: declares rust-version {declared} but "
                f"`{via}` in its dependencies needs {needed[0]}.{needed[1]}"
            )
    return sorted(violations)


def main() -> None:
    violations = rust_version_violations(json.loads(Path(sys.argv[1]).read_text()))
    if violations:
        print(
            "A crate declares a rust-version below what its dependencies declare; "
            "raise the crate's, or lower the dependency's:",
            file=sys.stderr,
        )
        for violation in violations:
            print(f"  - {violation}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()

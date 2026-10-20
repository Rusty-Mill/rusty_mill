#!/usr/bin/env python3
"""Fail when a workspace crate declares a `rust-version` below what its dependencies declare.

Read `cargo metadata` JSON from argv[1]. For every workspace member that declares
`rust-version`, use `cargo tree -p MEMBER` to obtain that member's independent
default-feature normal/build closure. Take the highest `rust-version` declared by any
package in that closure (an edition-2024 package counts as 1.85) and report the member
if its own declaration is lower. This compares declarations; it does not build at the
old toolchain.
"""
import json
import re
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

EDITION_2024_FLOOR = (1, 85, 0)


def _version(text: str | None) -> tuple[int, int, int]:
    """Parse and zero-pad `major[.minor[.patch]]`; no declaration is 0.0.0."""
    if not text:
        return (0, 0, 0)
    parts = [int(p) for p in text.split(".")]
    return tuple((parts + [0, 0])[:3])


def _floor(pkg: dict) -> tuple[int, int, int]:
    """The toolchain floor a package declares, or implies through its edition."""
    implied = EDITION_2024_FLOOR if pkg.get("edition") == "2024" else (0, 0, 0)
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
            default=((0, 0, 0), ""),
        )
        if needed > _version(declared):
            violations.append(
                f"{packages[member]['name']}: declares rust-version {declared} but "
                f"`{via}` in its dependencies needs {'.'.join(map(str, needed))}"
            )
    return sorted(violations)


def _default_package_keys(member: dict, workspace_root: Path, cargo: str = "cargo") -> set[tuple[str, str]]:
    """Return (name, version) keys from one member's default normal/build graph."""
    result = subprocess.run(
        [
            cargo,
            "tree",
            "--locked",
            "-p",
            member["name"],
            "--edges",
            "normal,build",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ],
        cwd=workspace_root,
        check=True,
        text=True,
        capture_output=True,
    )
    keys = set()
    for line in result.stdout.splitlines():
        match = re.match(r"^(\S+) v(\S+)", line)
        if match:
            keys.add((match.group(1), match.group(2)))
    return keys


def workspace_default_violations(metadata: dict, workspace_root: Path, cargo: str = "cargo") -> list[str]:
    """Check each member against its separately selected Cargo default-build graph."""
    packages = {pkg["id"]: pkg for pkg in metadata["packages"]}
    by_key: dict[tuple[str, str], list[dict]] = {}
    for package in metadata["packages"]:
        by_key.setdefault((package["name"], package["version"]), []).append(package)

    members = [
        packages[member_id]
        for member_id in metadata["workspace_members"]
        if packages[member_id].get("rust_version")
    ]
    with ThreadPoolExecutor(max_workers=8) as executor:
        graphs = executor.map(
            lambda member: _default_package_keys(member, workspace_root, cargo), members
        )

    violations = []
    for member, keys in zip(members, graphs, strict=True):
        declared = member["rust_version"]
        closure = [pkg for key in keys for pkg in by_key.get(key, [])]
        needed, via = max(
            ((_floor(pkg), pkg["name"]) for pkg in closure if pkg["id"] != member["id"]),
            default=((0, 0, 0), ""),
        )
        if needed > _version(declared):
            violations.append(
                f"{member['name']}: declares rust-version {declared} but "
                f"`{via}` in its dependencies needs {'.'.join(map(str, needed))}"
            )
    return sorted(violations)


def main() -> None:
    metadata = json.loads(Path(sys.argv[1]).read_text())
    workspace_root = Path(metadata["workspace_root"])
    violations = workspace_default_violations(metadata, workspace_root)
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

#!/usr/bin/env python3
"""Narrow which workspace crates a `Cargo.lock`-only change actually affects.

`plan`'s existing file-path-based `affected_crates.py` cannot attribute
`Cargo.lock` to any single crate -- it isn't owned by one -- so the `plan`
job previously treated ANY `Cargo.lock` change as workspace-wide and ran a
full sweep. That is overly conservative for the common case a lockfile
changes with no `Cargo.toml` edit at all: a plain `cargo update` bumping one
transitive dependency's version. This module computes the actual set of
workspace crates whose dependency closure includes a package whose lock
entry changed, using the *full* resolve graph (workspace members and every
external package), unlike `affected_crates.py`'s `reverse_dependencies`
which deliberately drops external packages from its graph (a dependency
version bump is exactly the case that graph doesn't cover).

Reads Cargo.lock via `tomllib` (stdlib, Python 3.11+, matching the
`ubuntu-latest`/`windows-latest` runner images' default Python).

The graph logic lives in importable functions so `test_lockfile_diff.py`
can exercise it against synthetic data without a real `Cargo.lock` or
`cargo metadata` invocation.
"""
import sys
import tomllib
from pathlib import Path

PackageKey = tuple[str, str]  # (name, version)
PackageSignature = tuple[str | None, str | None, tuple[str, ...]]


def lockfile_package_keys(lock_text: str) -> dict[PackageKey, PackageSignature]:
    """Map every `[[package]]` entry to its source, checksum, and edges."""
    doc = tomllib.loads(lock_text)
    result: dict[PackageKey, PackageSignature] = {}
    for pkg in doc.get("package", []):
        key = (pkg["name"], pkg["version"])
        if key in result:
            raise ValueError(f"ambiguous duplicate Cargo.lock identity: {key}")
        result[key] = (
            pkg.get("source"),
            pkg.get("checksum"),
            tuple(sorted(pkg.get("dependencies", []))),
        )
    return result


def changed_package_keys(old_lock_text: str, new_lock_text: str) -> set[PackageKey]:
    """Return every (name, version) added, removed, or changed in-place.

    A version bump shows up as one removed key and one added key (the
    version is part of the key) -- both are real, independent changes to
    what's in the graph, so both come back rather than being collapsed.
    """
    old = lockfile_package_keys(old_lock_text)
    new = lockfile_package_keys(new_lock_text)
    changed = set()
    for key in set(old) | set(new):
        if old.get(key) != new.get(key):
            changed.add(key)
    return changed


def full_dependency_closure(metadata: dict) -> dict[str, set[str]]:
    """pkg_id -> every pkg_id reachable via `deps`, workspace or external.

    Unlike `affected_crates.reverse_dependencies`, nothing is dropped here
    -- an external package is exactly what a lockfile-only change touches.
    """
    resolve_nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    def closure(pkg_id: str) -> set[str]:
        # Do not memoize recursive partial results: dev-dependency cycles can
        # otherwise cache a closure before another edge in the cycle is seen.
        seen = {pkg_id}
        pending = [pkg_id]
        result: set[str] = set()
        while pending:
            node = resolve_nodes.get(pending.pop())
            if not node:
                continue
            for dep in node["deps"]:
                dep_id = dep["pkg"]
                if dep_id in seen:
                    continue
                seen.add(dep_id)
                result.add(dep_id)
                pending.append(dep_id)
        return result

    return {pkg_id: closure(pkg_id) for pkg_id in resolve_nodes}


def reliable_lockfile_impact(
    metadata: dict, changed_keys: set[PackageKey]
) -> list[str] | None:
    """Return affected workspace packages, or None when the impact is unknown."""
    if not changed_keys:
        return None

    key_to_ids: dict[PackageKey, set[str]] = {}
    for pkg in metadata["packages"]:
        key_to_ids.setdefault((pkg["name"], pkg["version"]), set()).add(pkg["id"])
    changed_ids: set[str] = set()
    unmatched: list[PackageKey] = []
    for key in changed_keys:
        matched_ids = key_to_ids.get(key, set())
        if matched_ids:
            changed_ids |= matched_ids
        else:
            unmatched.append(key)
    for name, _version in unmatched:
        # A lock-only version replacement removes the old key from head
        # metadata, but the same package name has exactly one changed,
        # resolved replacement in head. Anything else (a removed dependency,
        # multiple candidates, or an unrelated unknown entry) is ambiguous.
        replacements = [
            key
            for key in changed_keys
            if key[0] == name and key in key_to_ids
        ]
        if len(replacements) != 1:
            return None
    if not changed_ids:
        return None

    closures = full_dependency_closure(metadata)
    workspace_ids = set(metadata["workspace_members"])
    packages_by_id = {pkg["id"]: pkg for pkg in metadata["packages"]}

    affected = {
        pkg_id
        for pkg_id in workspace_ids
        if pkg_id in changed_ids or (closures.get(pkg_id, set()) & changed_ids)
    }
    return sorted(packages_by_id[pkg_id]["name"] for pkg_id in affected)


def affected_by_lockfile_changes(metadata: dict, changed_keys: set[PackageKey]) -> list[str]:
    """Sorted workspace package names whose dependency closure includes a
    package matching one of `changed_keys` (or that a changed key names
    directly -- a workspace member's own version pin changing counts too).
    """
    if not changed_keys:
        return []
    impact = reliable_lockfile_impact(metadata, changed_keys)
    if impact is None:
        # A changed lockfile entry cargo metadata's resolve doesn't know
        # about at all (e.g. a dev-only/build-only dependency that
        # --all-features metadata still excludes, or a stale/partial
        # metadata snapshot) is exactly the "can't prove it's safe" case --
        # never silently narrow past it. Caller should fall back to a full
        # sweep when this returns something unexpected; returning every
        # workspace member here makes that fallback the safe default even
        # if a caller forgets the check.
        workspace_ids = set(metadata["workspace_members"])
        return sorted(
            pkg["name"] for pkg in metadata["packages"] if pkg["id"] in workspace_ids
        )

    return impact


def main() -> None:
    metadata_path, old_lock_path, new_lock_path = sys.argv[1], sys.argv[2], sys.argv[3]
    import json

    metadata = json.loads(Path(metadata_path).read_text())
    old_lock_text = Path(old_lock_path).read_text()
    new_lock_text = Path(new_lock_path).read_text()
    changed = changed_package_keys(old_lock_text, new_lock_text)
    impact = reliable_lockfile_impact(metadata, changed)
    if impact is None:
        print("Cargo.lock impact is not reliably represented by head metadata", file=sys.stderr)
        raise SystemExit(2)
    print(" ".join(impact))


if __name__ == "__main__":
    main()

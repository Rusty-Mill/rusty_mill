#!/usr/bin/env python3
"""Compute which workspace crates a set of changed files affects.

Reads changed file paths (one per line, repo-root-relative) from stdin and
the JSON from `cargo metadata --format-version=1 --all-features` at the
path given as argv[1]. Prints a space-separated list of affected workspace
package names to stdout: every crate that owns a changed file, plus every
crate that transitively depends on one of those (a dependency's behavior
can break its dependents' tests too). `--all-features` matters here so the
resolve graph includes edges that only exist behind an optional feature --
CI itself always builds with --all-features, so the affected-crate set
has to match that graph, not the default-features one.

A package outside the workspace but inside the repository (a `[patch]`ed
`vendor/*` crate) owns its files too: every workspace crate whose full
dependency closure includes it is selected, plus their dependents.

Printing nothing means no workspace crate needs Cargo validation.
Only the audited prose-only TLS ADR Markdown paths are excluded from seeds.

The graph logic lives in `affected_packages` so `test_affected_crates.py`
can exercise it against synthetic metadata without spawning cargo.
"""
import json
import sys
from pathlib import Path, PurePosixPath
from typing import Iterable, Optional

CrateDirs = list[tuple[Path, str]]


def is_prose_only_tls_adr(file_path: str) -> bool:
    """Match only audited TLS ADR Markdown, not other docs or build inputs."""
    # These decision records are linked in comments, never included/read by
    # Rust code, tests or build scripts. Keep this exception narrow: Markdown
    # elsewhere may be compiled with include_str! or consumed as test data.
    path = PurePosixPath(file_path)
    return (
        path.parent == PurePosixPath("crates/libs/net/rusty_tls/docs/adr")
        and path.suffix == ".md"
    )


def owning_crate(file_path: str, crate_dirs: CrateDirs, root_dir: Path) -> Optional[str]:
    """Return the package id whose directory contains `file_path`, or None.

    `crate_dirs` must be sorted longest directory first so a nested crate
    wins over the crate that contains it.
    """
    abs_path = (root_dir / file_path).resolve()
    for crate_dir, pkg_id in crate_dirs:
        try:
            abs_path.relative_to(crate_dir)
        except ValueError:
            continue
        return pkg_id
    return None


def local_package_dirs(metadata: dict, root_dir: Path) -> CrateDirs:
    """Map each non-member package whose manifest lives under `root_dir`, longest first."""
    workspace_ids = set(metadata["workspace_members"])
    local = {
        pkg["id"]: pkg
        for pkg in metadata["packages"]
        if pkg["id"] not in workspace_ids
        and Path(pkg["manifest_path"]).resolve().is_relative_to(root_dir)
    }
    return workspace_crate_dirs(local)


def local_package_consumers(metadata: dict, changed_local: set[str]) -> set[str]:
    """Workspace members that depend, directly or through any package, on a changed local package.

    Walks the full resolve graph backwards, external packages included, so a
    member that reaches the patched package only through a registry crate counts.
    """
    dependents: dict[str, set[str]] = {}
    for node in metadata["resolve"]["nodes"]:
        for dep in node["deps"]:
            dependents.setdefault(dep["pkg"], set()).add(node["id"])
    seen = set(changed_local)
    queue = list(changed_local)
    while queue:
        for dependent in dependents.get(queue.pop(), ()):
            if dependent not in seen:
                seen.add(dependent)
                queue.append(dependent)
    return seen & set(metadata["workspace_members"])


def workspace_crate_dirs(packages: dict[str, dict]) -> CrateDirs:
    """Map each workspace package to its manifest directory, longest first."""
    return sorted(
        (
            (Path(pkg["manifest_path"]).parent.resolve(), pkg_id)
            for pkg_id, pkg in packages.items()
        ),
        key=lambda item: len(str(item[0])),
        reverse=True,
    )


def reverse_dependencies(metadata: dict, packages: dict[str, dict]) -> dict[str, set[str]]:
    """Build dependency -> dependents over workspace members only.

    Edges come from the resolve graph, so they reflect the feature set the
    metadata was generated with. Dependencies outside the workspace are
    dropped: nothing in this repo needs to re-run because of them.
    """
    resolve_nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    reverse_deps: dict[str, set[str]] = {pkg_id: set() for pkg_id in packages}
    for pkg_id in packages:
        node = resolve_nodes.get(pkg_id)
        if not node:
            continue
        for dep in node["deps"]:
            dep_id = dep["pkg"]
            if dep_id in packages:
                reverse_deps[dep_id].add(pkg_id)
    return reverse_deps


def affected_packages(metadata: dict, changed_files: Iterable[str]) -> list[str]:
    """Return the sorted names of workspace packages affected by `changed_files`."""
    workspace_ids = set(metadata["workspace_members"])
    packages = {
        pkg["id"]: pkg for pkg in metadata["packages"] if pkg["id"] in workspace_ids
    }
    root_dir = Path(metadata["workspace_root"]).resolve()
    crate_dirs = workspace_crate_dirs(packages)

    seeds = [f for f in changed_files if not is_prose_only_tls_adr(f)]
    changed_ids = {
        owner
        for owner in (owning_crate(f, crate_dirs, root_dir) for f in seeds)
        if owner is not None
    }
    local_dirs = local_package_dirs(metadata, root_dir)
    changed_local = {
        owner
        for owner in (owning_crate(f, local_dirs, root_dir) for f in seeds)
        if owner is not None
    }
    changed_ids |= local_package_consumers(metadata, changed_local)
    if not changed_ids:
        return []

    reverse_deps = reverse_dependencies(metadata, packages)
    affected: set[str] = set()
    queue = list(changed_ids)
    while queue:
        pkg_id = queue.pop()
        if pkg_id in affected:
            continue
        affected.add(pkg_id)
        queue.extend(reverse_deps.get(pkg_id, ()))

    return sorted(packages[pkg_id]["name"] for pkg_id in affected)


def main() -> None:
    metadata = json.loads(Path(sys.argv[1]).read_text())
    changed_files = [line.strip() for line in sys.stdin if line.strip()]
    print(" ".join(affected_packages(metadata, changed_files)))


if __name__ == "__main__":
    main()

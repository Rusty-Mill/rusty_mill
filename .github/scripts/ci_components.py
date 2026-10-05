"""Partition an already-expanded Cargo impact set without losing any packages.

Applications get independent CI lanes, including their nested crates. Shared
libraries are grouped by layer/category. This changes scheduling only: the
affected-crates planner still decides which packages need validation.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
from collections.abc import Iterable


# Two OS jobs per scope must fit GitHub's 256-job matrix limit. Scoped runs
# have one shard; full sweeps retain the existing single scope and three shards.
MAX_COMPONENTS = 128


def component_name(manifest: str, workspace_root: str) -> str:
    """Return a stable lane for a workspace manifest, independent of the diff."""
    parts = Path(manifest).resolve().relative_to(Path(workspace_root).resolve()).parts
    if len(parts) >= 4 and parts[:2] in (("crates", "apps"), ("crates", "libs")):
        component = "--".join(parts[1:3])
    elif len(parts) >= 3 and parts[0] == "crates":
        component = parts[1]
    else:
        return "workspace-other"
    # Names enter shell log paths as well as artifact and concurrency keys.
    # Unusual directory names may share a lane, but cannot become shell input.
    return component if re.fullmatch(r"[A-Za-z0-9_.-]+", component) else "workspace-other"


def component_scopes(metadata: dict, selected: Iterable[str]) -> list[dict[str, str]]:
    """Partition exactly the selected packages; reject unknown package names."""
    members = set(metadata["workspace_members"])
    packages = {
        package["name"]: package
        for package in metadata["packages"]
        if package["id"] in members
    }
    selected = set(selected)
    unknown = selected - packages.keys()
    if unknown:
        raise ValueError(f"Selected packages absent from workspace metadata: {sorted(unknown)}")
    groups: dict[str, list[str]] = {}
    for name in sorted(selected):
        component = component_name(packages[name]["manifest_path"], metadata["workspace_root"])
        groups.setdefault(component, []).append(name)
    if len(groups) > MAX_COMPONENTS:
        # Preserve all selected tests if workspace growth exceeds the matrix
        # limit. A coarser lane costs parallelism, never coverage.
        groups = {"scoped-workspace": sorted(selected)}
    return [
        {"component": component, "packages": " ".join(names)}
        for component, names in sorted(groups.items())
    ]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("metadata", type=Path)
    parser.add_argument("--packages", required=True)
    args = parser.parse_args()
    metadata = json.loads(args.metadata.read_text(encoding="utf-8"))
    print(json.dumps(component_scopes(metadata, args.packages.split()), separators=(",", ":")))


if __name__ == "__main__":
    main()

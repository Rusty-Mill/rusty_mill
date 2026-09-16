#!/usr/bin/env python3
"""Enforce ADR-0003's workspace layers before any crate directories move.

Read cargo metadata JSON from argv[1]. Check every member's classification,
upward dependency edges of every kind, and app edges across product families.
"""
import json
import sys
from pathlib import Path, PurePosixPath


# docs/adr/0003-workspace-layout-by-layer.md, Appendix A (layer-order).
LAYER_ORDER = ("foundation", "platform", "libs", "apps", "tools")


def workspace_packages(metadata: dict) -> dict[str, dict]:
    """Index workspace members by their resolved package IDs."""
    members = set(metadata["workspace_members"])
    return {pkg["id"]: pkg for pkg in metadata["packages"] if pkg["id"] in members}


def workspace_edges(metadata: dict) -> set[tuple[str, str]]:
    """Return distinct caller/target member edges, including dev and build deps."""
    members = set(metadata["workspace_members"])
    return {
        (node["id"], dep["pkg"])
        for node in metadata["resolve"]["nodes"]
        if node["id"] in members
        for dep in node["deps"]
        if dep["pkg"] in members
    }


def package_layer(package: dict) -> str | None:
    """Read a member's layer, treating malformed metadata as missing."""
    metadata = package.get("metadata")
    table = metadata.get("rusty_mill") if isinstance(metadata, dict) else None
    layer = table.get("layer") if isinstance(table, dict) else None
    return layer if isinstance(layer, str) else None


def package_family(package: dict, workspace_root: str) -> str:
    """Find the family below workspace-root/crates on either host path syntax.

    Before ADR-0003 Phase 1, every member lived at `crates/<family>/...`, so
    the family was always the second path component. Once a family moves
    under its layer directory (`crates/<layer>/<family>/...`, e.g.
    `crates/platform/rustils/crates/platform`), that second component is
    the layer name instead -- skip it so `platform-linux`'s family stays
    `rustils`, not `platform` (which would collapse every crate in a layer
    into one fake "family" and silently disable the cross-family apps
    check). Both directory shapes coexist during the migration, so this
    must handle either one.
    """
    directory = PurePosixPath(package["manifest_path"].replace("\\", "/")).parent
    root = PurePosixPath(workspace_root.replace("\\", "/"))
    parts = directory.relative_to(root).parts
    if len(parts) < 2 or parts[0] != "crates":
        raise ValueError(f"{package['name']}: manifest directory must be under crates/<family>")
    if parts[1] in LAYER_ORDER and len(parts) >= 3:
        return parts[2]
    return parts[1]


def workspace_layer_violations(metadata: dict) -> list[str]:
    """Return sorted layer-policy violations against the unmoved workspace."""
    packages = workspace_packages(metadata)
    layers = {pkg_id: package_layer(pkg) for pkg_id, pkg in packages.items()}
    violations = [
        f"{pkg['name']}: missing or unrecognized rusty_mill.layer {layers[pkg_id]!r}; "
        f"expected one of {', '.join(LAYER_ORDER)}"
        for pkg_id, pkg in packages.items()
        if layers[pkg_id] not in LAYER_ORDER
    ]
    for caller_id, target_id in workspace_edges(metadata):
        caller, target = packages[caller_id], packages[target_id]
        caller_layer, target_layer = layers[caller_id], layers[target_id]
        if caller_layer not in LAYER_ORDER or target_layer not in LAYER_ORDER:
            continue
        edge = f"{caller['name']} ({caller_layer}) -> {target['name']} ({target_layer})"
        if LAYER_ORDER.index(target_layer) > LAYER_ORDER.index(caller_layer):
            violations.append(f"upward dependency: {edge}")
        if caller_layer == target_layer == "apps":
            root = metadata["workspace_root"]
            if package_family(caller, root) != package_family(target, root):
                violations.append(f"cross-family apps dependency: {edge}")
    return sorted(violations)


def main() -> None:
    """Read metadata and report one violation per line; success is silent."""
    metadata = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
    violations = workspace_layer_violations(metadata)
    for violation in violations:
        print(violation, file=sys.stderr)
    if violations:
        sys.exit(1)


if __name__ == "__main__":
    main()

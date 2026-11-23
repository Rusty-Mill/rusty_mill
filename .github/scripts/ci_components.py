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

# Crates whose build needs the apt packages in setup-build-env's
# native-packages.txt (X11/wayland, GTK/WebKit/Tauri, D-Bus/keyring, ALSA,
# protoc). A lane whose packages cannot reach one skips that install, which
# is a mirror download that has timed out CI. Other `-sys` crates in the
# workspace (ring, aws-lc, sqlite, zstd, openssl) build from bundled source
# or against libraries already on the runner image.
NATIVE_CRATES = re.compile(
    r"alsa-sys|cpal|libdbus-sys|dbus|dbus-secret-service|keyring|tauri|tray-icon"
    r"|libappindicator.*|(gtk|gdk|glib|gio|gobject|atk|pango|cairo)(-sys|-sys-rs)?"
    r"|gdk.*-sys|webkit2gtk.*|javascriptcore-rs-sys|soup3-sys|x11.*|xkb.*|wayland-.*"
    r"|rusty_a2a|rusty_gui"
)


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


def needs_native(metadata: dict, names: Iterable[str]) -> bool:
    """True when any named package's dependency closure includes a NATIVE_CRATES crate."""
    by_id = {package["id"]: package["name"] for package in metadata["packages"]}
    deps = {node["id"]: [d["pkg"] for d in node["deps"]] for node in metadata["resolve"]["nodes"]}
    wanted = set(names)
    stack = [i for i in metadata["workspace_members"] if by_id[i] in wanted]
    seen = set(stack)
    while stack:
        node = stack.pop()
        if NATIVE_CRATES.fullmatch(by_id[node]):
            return True
        for dep in deps.get(node, []):
            if dep not in seen:
                seen.add(dep)
                stack.append(dep)
    return False


def component_scopes(metadata: dict, selected: Iterable[str]) -> list[dict[str, str]]:
    """Partition exactly the selected packages; reject unknown package names.

    Each scope carries `native` ("true"/"false"): whether its lane must install
    the native apt packages (see NATIVE_CRATES)."""
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
        {
            "component": component,
            "packages": " ".join(names),
            "native": str(needs_native(metadata, names)).lower(),
        }
        for component, names in sorted(groups.items())
    ]


def covers_most_of_workspace(metadata: dict, scopes: list[dict[str, str]]) -> bool:
    """True when the scoped lanes span more than half of the workspace's components.

    Past that point the full sweep is cheaper: every lane rebuilds the shared
    dependencies for clippy and test, so #1066's 28 of 45 components ran 145
    jobs and ~350 runner-minutes, where a full sweep runs 58 jobs and ~270.
    """
    members = set(metadata["workspace_members"])
    components = {
        component_name(package["manifest_path"], metadata["workspace_root"])
        for package in metadata["packages"]
        if package["id"] in members
    }
    return 2 * len(scopes) > len(components)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("metadata", type=Path)
    parser.add_argument("--packages", required=True)
    parser.add_argument(
        "--full-if-most",
        action="store_true",
        help="print nothing and exit 3 when the lanes cover most of the workspace",
    )
    args = parser.parse_args()
    metadata = json.loads(args.metadata.read_text(encoding="utf-8"))
    scopes = component_scopes(metadata, args.packages.split())
    if args.full_if_most and covers_most_of_workspace(metadata, scopes):
        raise SystemExit(3)
    print(json.dumps(scopes, separators=(",", ":")))


if __name__ == "__main__":
    main()

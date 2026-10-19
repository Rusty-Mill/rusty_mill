"""Read-only repository intelligence; policy and graph helpers come from CI.

Loaded with bundled first-party modules by the Rust composition root, never
from the inspected repository. Requires Python 3.11+ (stdlib only).
"""
import hashlib
import json
from pathlib import Path, PurePath
import subprocess
import sys
import tomllib
from typing import Callable

from affected_crates import reverse_dependencies
from check_workspace_layers import (
    LAYER_ORDER, package_family, package_layer, workspace_edges,
    workspace_layer_violations, workspace_packages,
)
from check_workspace_deps import (
    git_shadowed_workspace_members, pinned_dependency_violations,
)


def closure(seeds: list[str], reverse: dict[str, set[str]]) -> list[str]:
    """Conservative transitive test impact, including dev/build/optional edges."""
    seen: set[str] = set()
    pending = list(seeds)
    while pending:
        member = pending.pop()
        if member in seen:
            continue
        seen.add(member)
        pending.extend(reverse.get(member, set()) - seen)
    return sorted(seen)


def canonical_path(value: str) -> Path:
    """Resolve boundaries using one spelling for Windows extended-length paths."""
    path = str(Path(value).resolve())
    if sys.platform == "win32":
        path = ("\\\\" + path[8:]) if path.startswith("\\\\?\\UNC\\") else path.removeprefix("\\\\?\\")
    return Path(path)


def finding_identity(rule: str, evidence: list[dict], root: PurePath) -> str:
    """Normalize native paths and Cargo file URLs before JSON escaping, for IDs only."""
    variants = sorted({str(root), root.as_posix(), root.as_uri()}, key=len, reverse=True)

    def normalize(value: object) -> object:
        if isinstance(value, str):
            for variant in variants:
                value = value.replace(variant, "<workspace>")
            return value
        if isinstance(value, list):
            return [normalize(item) for item in value]
        if isinstance(value, dict):
            return {key: normalize(item) for key, item in value.items()}
        return value

    return json.dumps([rule, normalize(evidence)], sort_keys=True, separators=(",", ":"))


def analyze(metadata: dict) -> dict:
    """Produce versioned findings without executing tests, agents or changes."""
    packages = workspace_packages(metadata)
    if set(packages) != set(metadata["workspace_members"]):
        raise ValueError("metadata is missing workspace packages")
    nodes = (metadata.get("resolve") or {}).get("nodes", [])
    if not set(packages).issubset({n["id"] for n in nodes}):
        raise ValueError("complete Cargo resolve graph required; --no-deps is insufficient")
    root = canonical_path(metadata["workspace_root"])
    # Validate manifest boundaries before reading any manifest. A workspace
    # member outside the root is unsupported rather than silently omitted.
    manifests = {
        key: canonical_path(pkg["manifest_path"]).relative_to(root).as_posix()
        for key, pkg in packages.items()
    }
    # Cargo may mix ordinary and extended-length Windows paths in one result.
    # Feed the policy helpers the same canonical spelling used for boundaries.
    metadata = dict(metadata, workspace_root=str(root), packages=[
        dict(pkg, manifest_path=str(root / manifests[pkg["id"]])) if pkg["id"] in packages else pkg
        for pkg in metadata["packages"]
    ])
    packages = workspace_packages(metadata)
    reverse = reverse_dependencies(metadata, packages)
    edges = workspace_edges(metadata)
    dependency_details = {
        (node["id"], dep["pkg"]): {
            "alias": dep["name"],
            "kinds": sorted(dep.get("dep_kinds", []), key=lambda k: (k.get("kind") or "", k.get("target") or "")),
        }
        for node in nodes for dep in node["deps"]
        if node["id"] in packages and dep["pkg"] in packages
    }
    findings: list[dict] = []

    def finding(rule: str, ids: list[str], evidence: list[dict], recommendation: str) -> None:
        ids = sorted(set(ids))
        identity = finding_identity(rule, evidence, root)
        findings.append({
            "id": rule + ":" + hashlib.sha256(identity.encode()).hexdigest()[:20],
            "rule": rule,
            "severity": "warning" if rule == "duplicate-declaration" else "error",
            "affected_crates": ids,
            "evidence": evidence,
            "recommendation": recommendation,
            "impact": {
                "method": "transitive reverse workspace dependencies; all features and dependency kinds",
                "crates_to_validate": closure(ids, reverse),
                "estimate": "conservative validation scope, not edit size or runtime risk",
            },
        })

    # Call the canonical validator on single-member / single-edge projections
    # so each diagnostic has exact crate ownership without parsing prose.
    for key, pkg in sorted(packages.items()):
        single = dict(metadata, workspace_members=[key], resolve={"nodes": []})
        for message in workspace_layer_violations(single):
            finding("layer-classification", [key], [{"manifest": manifests[key],
                    "field": "package.metadata.rusty_mill.layer", "message": message}],
                    "Declare a recognized architectural layer under ADR-0003.")
        for message in pinned_dependency_violations(single):
            finding("pinned-external-dependency", [key], [{"manifest": manifests[key],
                    "message": message}], "Reconcile the manifest with ADR-0002's external dependency allow-list.")
    for caller, target in sorted(edges):
        projection = dict(metadata, workspace_members=[caller, target],
                          resolve={"nodes": [{"id": caller, "deps": [{"pkg": target}]}]})
        for message in workspace_layer_violations(projection):
            if not message.startswith(("upward dependency:", "cross-family apps dependency:")):
                continue
            finding("dependency-layer", [caller, target], [{
                "manifest": manifests[caller], "dependency": target,
                "declaration": dependency_details[caller, target], "message": message}],
                "Remove or invert the forbidden dependency; preserve product-family boundaries (ADR-0003).")
    resolve_reverse: dict[str, set[str]] = {}
    for node in nodes:
        for dep in node["deps"]:
            resolve_reverse.setdefault(dep["pkg"], set()).add(node["id"])
    for message in git_shadowed_workspace_members(metadata):
        name = message.split(" (resolved from ", 1)[0]
        shadow_ids = {p["id"] for p in metadata["packages"]
                      if p["name"] == name and p["id"] not in packages}
        consumers = [key for key in closure(list(shadow_ids), resolve_reverse) if key in packages]
        ids = consumers + [k for k, p in packages.items() if p["name"] == name]
        finding("first-party-git-shadow", ids, [{"message": message}],
                "Resolve the first-party dependency through the local workspace (ATLAS-RWC-0050).")
    duplicate_declarations(packages, manifests, root, finding)
    crates = [{
        "id": key, "name": pkg["name"], "manifest": manifests[key],
        "layer": package_layer(pkg), "family": package_family(pkg, str(root)),
        "dependencies": sorted(t for c, t in edges if c == key),
        "reverse_dependencies": sorted(reverse[key]),
    } for key, pkg in sorted(packages.items())]
    return {
        "schema_version": 1,
        "workspace_root": str(root),
        "analysis": {"metadata": "cargo metadata --format-version=1 --all-features --frozen",
                     "layers": list(LAYER_ORDER), "executes_changes": False,
                     "validation": "static policy checks only; no build, test or CI status inferred"},
        "crates": crates,
        "dependency_edges": [{"caller": caller, "dependency": target,
                              **dependency_details[caller, target]}
                             for caller, target in sorted(edges)],
        "findings": sorted(findings, key=lambda f: f["id"]),
    }


def duplicate_declarations(packages: dict, manifests: dict, root: Path, emit: Callable[[str, list[str], list[dict], str], None]) -> None:
    """Suggest hoisting identical explicit external declarations across members.

    Equal declarations include kind, target, features, source and version. Path
    and inherited declarations are excluded; this is a review suggestion, not
    proof that semantically different declarations can be merged.
    """
    groups: dict[str, list[tuple[str, dict]]] = {}
    for key in sorted(packages):
        document = tomllib.loads((root / manifests[key]).read_text(encoding="utf-8"))
        tables = [("", document)] + sorted(document.get("target", {}).items())
        for target, table in tables:
            for kind in ("dependencies", "dev-dependencies", "build-dependencies"):
                for name, value in sorted(table.get(kind, {}).items()):
                    declaration = {"version": value} if isinstance(value, str) else value
                    if "workspace" in declaration or "path" in declaration:
                        continue
                    signature = json.dumps([target, kind, name, declaration], sort_keys=True)
                    groups.setdefault(signature, []).append((key, {
                        "manifest": manifests[key], "table": f"target.{target}.{kind}" if target else kind,
                        "dependency": name, "declaration": declaration,
                    }))
    for entries in groups.values():
        if len({key for key, _ in entries}) < 2:
            continue
        emit("duplicate-declaration", [key for key, _ in entries],
             [evidence for _, evidence in entries],
             "Consider hoisting identical declarations to workspace.dependencies; review feature and source semantics before editing.")


def main() -> None:
    """Cargo frozen forbids lock updates and network; metadata never builds code."""
    result = subprocess.run(
        ["cargo", "metadata", "--format-version=1", "--all-features", "--frozen"],
        cwd=sys.argv[1], capture_output=True, timeout=90, check=False,
    )
    if result.returncode:
        raise ValueError("cargo metadata failed (requires a current lockfile and cached dependencies): "
                         + result.stderr.decode(errors="replace")[:1000])
    print(json.dumps(analyze(json.loads(result.stdout)), sort_keys=True, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, TypeError, OSError, subprocess.TimeoutExpired) as error:
        print(f"repository inspection failed: {error}", file=sys.stderr)
        sys.exit(1)

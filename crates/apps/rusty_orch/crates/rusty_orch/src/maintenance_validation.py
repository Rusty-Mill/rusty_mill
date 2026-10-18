"""Frozen BBP pre-validation: verify the candidate is exactly the bounded repair.

Runs in the existing sandboxed runner checkout, before Cargo profiles. No target
repository code is imported. Git HEAD is the candidate's recorded base.
"""
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tomllib


def git(*args):
    # Inspection concerns the candidate, not machine/user Git configuration.
    # System config may also be deliberately unreadable inside the sandbox.
    environment = dict(os.environ, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL="/dev/null")
    return subprocess.check_output(["git", *args], env=environment)


def before(path):
    return tomllib.loads(git("show", "HEAD:" + path).decode())


def after(path):
    return tomllib.loads(Path(path).read_text())


def table(document, evidence):
    name = evidence["table"]
    if name.startswith("target."):
        target, kind = name[7:].rsplit(".", 1)
        return document["target"][target][kind]
    return document[name]


def verify(selected):
    changed = set(git("diff", "--cached", "--name-only", "-z").decode().strip("\0").split("\0"))
    if not changed or "" in changed or not changed <= set(selected["allowed_paths"]):
        raise ValueError("candidate path scope differs from maintenance selection")
    # Reject deletion, mode changes, symlinks, renames and binary modifications.
    for line in git("diff", "--cached", "--raw", "--no-abbrev").decode().splitlines():
        fields = line.split()
        if fields[0] != ":100644" or fields[1] != "100644" or fields[4] != "M":
            raise ValueError("only modifications of regular manifests are allowed")
    if selected["rule"] == "layer-classification":
        for path in changed:
            old, new = before(path), after(path)
            expected = path.split("/")[1]
            package = new["package"]
            current = package["metadata"]["rusty_mill"].pop("layer")
            original = old["package"].get("metadata", {}).get("rusty_mill", {}).get("layer")
            if current != expected or original == expected:
                raise ValueError("layer repair must restore the directory layer")
            # Normalize empty metadata tables introduced solely by the repair.
            for document in (old, new):
                metadata = document["package"].get("metadata", {})
                mill = metadata.get("rusty_mill", {})
                mill.pop("layer", None)
                if not mill:
                    metadata.pop("rusty_mill", None)
                if not metadata:
                    document["package"].pop("metadata", None)
            if old != new:
                raise ValueError("layer repair changed unrelated manifest semantics")
        return
    if selected["rule"] != "duplicate-declaration":
        raise ValueError("unsupported maintenance rule")
    old_root, new_root = before("Cargo.toml"), after("Cargo.toml")
    expected_root = copy.deepcopy(old_root)
    old_members, expected_members = {}, {}
    for evidence in selected["evidence"]:
        path, name, declaration = evidence["manifest"], evidence["dependency"], evidence["declaration"]
        if path not in selected["allowed_paths"] or path == "Cargo.toml":
            raise ValueError("invalid duplicate declaration evidence")
        if path not in old_members:
            old_members[path] = before(path)
            expected_members[path] = copy.deepcopy(old_members[path])
        original = table(old_members[path], evidence)[name]
        if isinstance(original, str):
            original = {"version": original}
        if original != declaration or "path" in declaration or "workspace" in declaration:
            raise ValueError("duplicate declaration evidence is stale")
        dependencies = expected_root["workspace"].setdefault("dependencies", {})
        existing = dependencies.get(name)
        if isinstance(existing, str):
            existing = {"version": existing}
        if existing is not None and existing != declaration:
            raise ValueError("hoist would alter existing workspace dependency semantics")
        dependencies[name] = declaration
        table(expected_members[path], evidence)[name] = {"workspace": True}
    if len(old_members) < 2 or not changed.issuperset(old_members):
        raise ValueError("hoist must simplify at least two member manifests")
    if new_root != expected_root:
        raise ValueError("unexpected workspace dependency changes")
    for path, expected in expected_members.items():
        if after(path) != expected:
            raise ValueError("hoist changed unrelated member semantics")


if __name__ == "__main__":
    try:
        verify(json.loads(sys.argv[1]))
    except (ValueError, KeyError, TypeError, OSError, subprocess.CalledProcessError) as error:
        print("maintenance scope validation failed: " + str(error), file=sys.stderr)
        sys.exit(1)

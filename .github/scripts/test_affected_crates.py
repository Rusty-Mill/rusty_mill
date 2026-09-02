"""Unit tests for affected_crates.py against synthetic cargo metadata.

Run from the repo root:
    python3 -m unittest discover -s .github/scripts -p 'test_*.py'
"""
import unittest
from pathlib import Path

from affected_crates import affected_packages

ROOT = "/repo"


def metadata(crates: dict[str, tuple[str, list[str]]], external: list[str] = ()) -> dict:
    """Build the subset of `cargo metadata` that affected_crates.py reads.

    `crates` maps a package name to (directory relative to the root, list of
    dependency package names). Names double as package ids. `external` lists
    packages that are in the resolve graph but not workspace members.
    """
    packages = [
        {"id": name, "name": name, "manifest_path": f"{ROOT}/{directory}/Cargo.toml"}
        for name, (directory, _) in crates.items()
    ]
    packages += [{"id": name, "name": name, "manifest_path": f"/registry/{name}/Cargo.toml"} for name in external]
    nodes = [
        {"id": name, "deps": [{"name": dep, "pkg": dep} for dep in deps]}
        for name, (_, deps) in crates.items()
    ]
    nodes += [{"id": name, "deps": []} for name in external]
    return {
        "workspace_root": ROOT,
        "workspace_members": list(crates),
        "packages": packages,
        "resolve": {"nodes": nodes},
    }


class OwningCrateTests(unittest.TestCase):
    def test_file_inside_a_crate_selects_that_crate(self) -> None:
        md = metadata({"a": ("crates/a", []), "b": ("crates/b", [])})
        self.assertEqual(affected_packages(md, ["crates/a/src/lib.rs"]), ["a"])

    def test_file_outside_every_crate_selects_nothing(self) -> None:
        md = metadata({"a": ("crates/a", [])})
        self.assertEqual(affected_packages(md, ["README.md", "docs/adr/0001.md"]), [])

    def test_no_changed_files_selects_nothing(self) -> None:
        md = metadata({"a": ("crates/a", [])})
        self.assertEqual(affected_packages(md, []), [])

    def test_nested_crate_wins_over_its_parent(self) -> None:
        md = metadata({"outer": ("crates/outer", []), "inner": ("crates/outer/inner", [])})
        self.assertEqual(affected_packages(md, ["crates/outer/inner/src/lib.rs"]), ["inner"])
        self.assertEqual(affected_packages(md, ["crates/outer/src/lib.rs"]), ["outer"])

    def test_directory_name_prefix_is_not_containment(self) -> None:
        md = metadata({"foo": ("crates/foo", []), "foobar": ("crates/foobar", [])})
        self.assertEqual(affected_packages(md, ["crates/foobar/src/lib.rs"]), ["foobar"])

    def test_manifest_itself_counts_as_inside_the_crate(self) -> None:
        md = metadata({"a": ("crates/a", [])})
        self.assertEqual(affected_packages(md, ["crates/a/Cargo.toml"]), ["a"])


class ReverseDependencyTests(unittest.TestCase):
    def test_direct_dependents_are_included(self) -> None:
        md = metadata({"a": ("crates/a", []), "b": ("crates/b", ["a"]), "c": ("crates/c", [])})
        self.assertEqual(affected_packages(md, ["crates/a/src/lib.rs"]), ["a", "b"])

    def test_dependents_are_transitive(self) -> None:
        md = metadata({"a": ("crates/a", []), "b": ("crates/b", ["a"]), "c": ("crates/c", ["b"])})
        self.assertEqual(affected_packages(md, ["crates/a/src/lib.rs"]), ["a", "b", "c"])

    def test_changing_a_leaf_does_not_pull_in_its_dependencies(self) -> None:
        md = metadata({"a": ("crates/a", []), "b": ("crates/b", ["a"])})
        self.assertEqual(affected_packages(md, ["crates/b/src/lib.rs"]), ["b"])

    def test_external_dependencies_are_ignored(self) -> None:
        md = metadata({"a": ("crates/a", ["serde"]), "b": ("crates/b", ["serde"])}, external=["serde"])
        self.assertEqual(affected_packages(md, ["crates/a/src/lib.rs"]), ["a"])

    def test_dependency_cycles_terminate(self) -> None:
        # A dev-dependency back-edge makes the resolve graph cyclic.
        md = metadata({"a": ("crates/a", ["b"]), "b": ("crates/b", ["a"])})
        self.assertEqual(affected_packages(md, ["crates/a/src/lib.rs"]), ["a", "b"])

    def test_result_is_sorted_and_deduplicated(self) -> None:
        md = metadata({"z": ("crates/z", []), "m": ("crates/m", ["z"]), "a": ("crates/a", ["z"])})
        changed = ["crates/z/src/lib.rs", "crates/z/src/other.rs", "crates/a/src/lib.rs"]
        self.assertEqual(affected_packages(md, changed), ["a", "m", "z"])

    def test_workspace_member_missing_from_resolve_graph_is_tolerated(self) -> None:
        md = metadata({"a": ("crates/a", []), "b": ("crates/b", ["a"])})
        md["resolve"]["nodes"] = [n for n in md["resolve"]["nodes"] if n["id"] != "b"]
        self.assertEqual(affected_packages(md, ["crates/a/src/lib.rs"]), ["a"])


if __name__ == "__main__":
    unittest.main()

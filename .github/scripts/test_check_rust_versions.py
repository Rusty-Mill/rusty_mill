"""Unit tests for check_rust_versions.py against synthetic cargo metadata.

Run from the repo root:
    python3 -m unittest discover -s .github/scripts -p 'test_*.py'
"""
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

from check_rust_versions import rust_version_violations, workspace_default_violations


def pkg(name: str, rust_version: str | None = None, edition: str = "2021") -> dict:
    return {"id": name, "name": name, "rust_version": rust_version, "edition": edition}


def edges(*pairs: tuple[str, str, str | None]) -> dict[str, list[dict]]:
    """(caller, callee, kind) triples; kind None is a normal dependency."""
    out: dict[str, list[dict]] = {}
    for caller, callee, kind in pairs:
        out.setdefault(caller, []).append({"pkg": callee, "dep_kinds": [{"kind": kind}]})
    return out


def metadata(members: list[dict], others: list[dict], deps: dict[str, list[dict]]) -> dict:
    packages = members + others
    return {
        "workspace_members": [p["id"] for p in members],
        "packages": packages,
        "resolve": {"nodes": [{"id": p["id"], "deps": deps.get(p["id"], [])} for p in packages]},
    }


class RustVersionViolationsTests(unittest.TestCase):
    def test_a_crate_at_or_above_its_dependencies_is_clean(self) -> None:
        md = metadata([pkg("a", "1.88")], [pkg("b", "1.88"), pkg("c", "1.70")], edges(("a", "b", None), ("a", "c", None)))
        self.assertEqual(rust_version_violations(md), [])

    def test_a_crate_below_a_direct_dependency_is_flagged_with_the_dependency(self) -> None:
        md = metadata([pkg("a", "1.85")], [pkg("b", "1.88")], edges(("a", "b", None)))
        (violation,) = rust_version_violations(md)
        self.assertIn("a: declares rust-version 1.85", violation)
        self.assertIn("`b`", violation)
        self.assertIn("1.88", violation)

    def test_a_transitive_dependency_counts(self) -> None:
        md = metadata([pkg("a", "1.75")], [pkg("b"), pkg("c", "1.90")], edges(("a", "b", None), ("b", "c", None)))
        self.assertEqual(len(rust_version_violations(md)), 1)

    def test_build_dependencies_count_and_dev_dependencies_do_not(self) -> None:
        md = metadata([pkg("a", "1.75")], [pkg("b", "1.88")], edges(("a", "b", "build")))
        self.assertEqual(len(rust_version_violations(md)), 1)
        md = metadata([pkg("a", "1.75")], [pkg("b", "1.88")], edges(("a", "b", "dev")))
        self.assertEqual(rust_version_violations(md), [])

    def test_a_crate_that_declares_nothing_is_not_checked(self) -> None:
        md = metadata([pkg("a")], [pkg("b", "1.90")], edges(("a", "b", None)))
        self.assertEqual(rust_version_violations(md), [])

    def test_an_edition_2024_dependency_implies_1_85(self) -> None:
        md = metadata([pkg("a", "1.82")], [pkg("b", None, "2024")], edges(("a", "b", None)))
        (violation,) = rust_version_violations(md)
        self.assertIn("needs 1.85", violation)

    def test_zero_patch_is_equivalent_to_an_omitted_patch(self) -> None:
        md = metadata([pkg("a", "1.88")], [pkg("b", "1.88.0")], edges(("a", "b", None)))
        self.assertEqual(rust_version_violations(md), [])

    def test_direct_patch_level_requirement_is_enforced(self) -> None:
        md = metadata([pkg("a", "1.85")], [pkg("b", "1.85.1")], edges(("a", "b", None)))
        (violation,) = rust_version_violations(md)
        self.assertIn("needs 1.85.1", violation)

    def test_transitive_patch_level_requirement_is_enforced(self) -> None:
        md = metadata(
            [pkg("a", "1.85")],
            [pkg("b", "1.85"), pkg("c", "1.85.2")],
            edges(("a", "b", None), ("b", "c", None)),
        )
        (violation,) = rust_version_violations(md)
        self.assertIn("needs 1.85.2", violation)

    def test_a_cycle_through_workspace_members_terminates(self) -> None:
        md = metadata([pkg("a", "1.85"), pkg("b", "1.85")], [], edges(("a", "b", None), ("b", "a", "build")))
        self.assertEqual(rust_version_violations(md), [])

    def test_violations_are_sorted_by_crate(self) -> None:
        md = metadata(
            [pkg("z", "1.70"), pkg("a", "1.70")], [pkg("d", "1.88")], edges(("z", "d", None), ("a", "d", None))
        )
        names = [v.split(":")[0] for v in rust_version_violations(md)]
        self.assertEqual(names, ["a", "z"])

    def test_real_workspace_keeps_other_members_features_out_of_default_graph(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["plain", "consumer", "backend"]\nresolver = "2"\n'
            )
            for name in ("plain", "consumer", "backend"):
                (root / name / "src").mkdir(parents=True)
                (root / name / "src/lib.rs").write_text("")
            (root / "backend/Cargo.toml").write_text(
                '[package]\nname="backend"\nversion="0.1.0"\nedition="2021"\nrust-version="1.90"\n'
            )
            (root / "plain/Cargo.toml").write_text(
                '[package]\nname="plain"\nversion="0.1.0"\nedition="2021"\nrust-version="1.85"\n'
                '[features]\ndefault=[]\nhigh=["dep:backend"]\n[dependencies]\nbackend={path="../backend",optional=true}\n'
            )
            (root / "consumer/Cargo.toml").write_text(
                '[package]\nname="consumer"\nversion="0.1.0"\nedition="2021"\nrust-version="1.90"\n'
                '[dependencies]\nplain={path="../plain",features=["high"]}\n'
            )
            cargo = "cargo"
            subprocess.run([cargo, "generate-lockfile"], cwd=root, check=True, capture_output=True)
            raw = subprocess.run(
                [cargo, "metadata", "--format-version=1"], cwd=root, check=True, capture_output=True, text=True
            )
            md = json.loads(raw.stdout)
            self.assertEqual(workspace_default_violations(md, root, cargo), [])


if __name__ == "__main__":
    unittest.main()

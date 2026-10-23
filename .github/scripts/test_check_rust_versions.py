"""Unit tests for check_rust_versions.py against synthetic cargo metadata.

Run from the repo root:
    python3 -m unittest discover -s .github/scripts -p 'test_*.py'
"""
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from check_rust_versions import rust_version_violations, undeclared_members, workspace_default_violations


SCRIPT = Path(__file__).with_name("check_rust_versions.py")


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


class UndeclaredMembersTests(unittest.TestCase):
    def test_lists_only_members_without_a_declaration_sorted(self) -> None:
        md = metadata([pkg("z"), pkg("a"), pkg("ok", "1.75")], [pkg("dep")], {})
        self.assertEqual(undeclared_members(md), ["a", "z"])

    def test_all_declared_is_clean(self) -> None:
        self.assertEqual(undeclared_members(metadata([pkg("a", "1.75")], [], {})), [])


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


class ProductionPathTests(unittest.TestCase):
    def _run_fixture(self, manifests: dict[str, str]) -> subprocess.CompletedProcess[str]:
        """Run the checker entry point against a real temporary Cargo workspace."""
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        members = ", ".join(f'"{name}"' for name in manifests)
        (root / "Cargo.toml").write_text(
            f'[workspace]\nmembers = [{members}]\nresolver = "2"\n'
        )
        for name, manifest in manifests.items():
            (root / name / "src").mkdir(parents=True)
            (root / name / "src/lib.rs").write_text("")
            (root / name / "Cargo.toml").write_text(manifest)
        subprocess.run(["cargo", "generate-lockfile"], cwd=root, check=True, capture_output=True)
        metadata_path = root / "metadata.json"
        with metadata_path.open("w") as output:
            subprocess.run(
                ["cargo", "metadata", "--format-version=1"],
                cwd=root,
                check=True,
                stdout=output,
                stderr=subprocess.PIPE,
                text=True,
            )
        return subprocess.run(
            [sys.executable, os.fspath(SCRIPT), os.fspath(metadata_path)],
            cwd=root,
            capture_output=True,
            text=True,
        )

    @staticmethod
    def _package(name: str, rust_version: str, extra: str = "") -> str:
        return (
            f'[package]\nname="{name}"\nversion="0.1.0"\n'
            f'edition="2021"\nrust-version="{rust_version}"\n{extra}'
        )

    def test_main_flags_direct_patch_floor(self) -> None:
        result = self._run_fixture(
            {
                "app": self._package(
                    "app", "1.85", '[dependencies]\ndep={path="../dep"}\n'
                ),
                "dep": self._package("dep", "1.85.1"),
            }
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("app: declares rust-version 1.85", result.stderr)
        self.assertIn("needs 1.85.1", result.stderr)

    def test_main_flags_transitive_patch_floor(self) -> None:
        result = self._run_fixture(
            {
                "app": self._package(
                    "app", "1.85", '[dependencies]\nintermediate={path="../intermediate"}\n'
                ),
                "intermediate": self._package(
                    "intermediate", "1.85", '[dependencies]\ndep={path="../dep"}\n'
                ),
                "dep": self._package("dep", "1.85.2"),
            }
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("app: declares rust-version 1.85", result.stderr)
        self.assertIn("needs 1.85.2", result.stderr)

    def test_main_flags_build_dependency_floor(self) -> None:
        result = self._run_fixture(
            {
                "app": self._package(
                    "app", "1.85", '[build-dependencies]\nbuild_dep={path="../build_dep"}\n'
                ),
                "build_dep": self._package("build_dep", "1.88"),
            }
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("app: declares rust-version 1.85", result.stderr)
        self.assertIn("`build_dep`", result.stderr)

    def test_main_flags_windows_only_dependency_on_linux(self) -> None:
        result = self._run_fixture(
            {
                "app": self._package(
                    "app",
                    "1.85",
                    '[target.\'cfg(windows)\'.dependencies]\nwindows_dep={path="../windows_dep"}\n',
                ),
                "windows_dep": self._package("windows_dep", "1.88"),
            }
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("app: declares rust-version 1.85", result.stderr)
        self.assertIn("`windows_dep`", result.stderr)


if __name__ == "__main__":
    unittest.main()

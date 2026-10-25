"""Unit tests for lockfile_diff.py against synthetic Cargo.lock text and
cargo metadata.

Run from the repo root:
    python3 -m unittest discover -s .github/scripts -p 'test_*.py'
"""
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from lockfile_diff import (
    affected_by_lockfile_changes,
    changed_package_keys,
    full_dependency_closure,
    lockfile_package_keys,
    reliable_lockfile_impact,
)

ROOT = "/repo"
SCRIPT = Path(__file__).with_name("lockfile_diff.py")


def lock(entries: list[tuple[str, str, str | None, str | None]]) -> str:
    """Build minimal Cargo.lock TOML text from (name, version, source, checksum) rows."""
    blocks = []
    for name, version, source, checksum in entries:
        lines = ["[[package]]", f'name = "{name}"', f'version = "{version}"']
        if source is not None:
            lines.append(f'source = "{source}"')
        if checksum is not None:
            lines.append(f'checksum = "{checksum}"')
        blocks.append("\n".join(lines))
    return "\n\n".join(blocks) + "\n"


def metadata(crates: dict[str, tuple[str, list[str]]], external: dict[str, str]) -> dict:
    """`crates`: name -> (dir, [dep names]). `external`: name -> version."""
    packages = [
        {
            "id": name,
            "name": name,
            "version": "0.0.0",
            "manifest_path": f"{ROOT}/{directory}/Cargo.toml",
        }
        for name, (directory, _) in crates.items()
    ]
    packages += [
        {"id": name, "name": name, "version": version, "manifest_path": f"/registry/{name}/Cargo.toml"}
        for name, version in external.items()
    ]
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


class LockfilePackageKeysTests(unittest.TestCase):
    def test_parses_name_version_source_checksum(self) -> None:
        text = lock([("serde", "1.0.0", "registry+https://x", "abc123")])
        self.assertEqual(
            lockfile_package_keys(text),
            {("serde", "1.0.0"): ("registry+https://x", "abc123", ())},
        )

    def test_missing_source_and_checksum_are_none(self) -> None:
        text = lock([("local_crate", "0.1.0", None, None)])
        self.assertEqual(lockfile_package_keys(text), {("local_crate", "0.1.0"): (None, None, ())})

    def test_duplicate_name_version_with_different_sources_is_ambiguous(self) -> None:
        text = """[[package]]
name = "same"
version = "1.0.0"
source = "registry+https://example.invalid"

[[package]]
name = "same"
version = "1.0.0"
source = "git+https://example.invalid/same"
"""
        with self.assertRaisesRegex(ValueError, "ambiguous duplicate"):
            lockfile_package_keys(text)

    def test_dependency_list_change_is_detected(self) -> None:
        old = '''[[package]]
name = "app"
version = "1.0.0"
dependencies = ["dep_a 1.0.0"]
'''
        new = '''[[package]]
name = "app"
version = "1.0.0"
dependencies = ["dep_b 1.0.0"]
'''
        self.assertEqual(changed_package_keys(old, new), {("app", "1.0.0")})


class ChangedPackageKeysTests(unittest.TestCase):
    def test_identical_lockfiles_change_nothing(self) -> None:
        text = lock([("serde", "1.0.0", "registry+https://x", "abc")])
        self.assertEqual(changed_package_keys(text, text), set())

    def test_version_bump_reports_both_old_and_new_keys(self) -> None:
        old = lock([("serde", "1.0.0", "registry+https://x", "abc")])
        new = lock([("serde", "1.0.1", "registry+https://x", "def")])
        self.assertEqual(
            changed_package_keys(old, new),
            {("serde", "1.0.0"), ("serde", "1.0.1")},
        )

    def test_checksum_change_at_the_same_version_is_detected(self) -> None:
        old = lock([("serde", "1.0.0", "registry+https://x", "abc")])
        new = lock([("serde", "1.0.0", "registry+https://x", "different")])
        self.assertEqual(changed_package_keys(old, new), {("serde", "1.0.0")})

    def test_source_change_at_the_same_version_is_detected(self) -> None:
        old = lock([("serde", "1.0.0", "registry+https://x", "abc")])
        new = lock([("serde", "1.0.0", "git+https://github.com/serde-rs/serde", None)])
        self.assertEqual(changed_package_keys(old, new), {("serde", "1.0.0")})

    def test_added_and_removed_packages_are_both_detected(self) -> None:
        old = lock([("a", "1.0.0", None, None)])
        new = lock([("a", "1.0.0", None, None), ("b", "2.0.0", None, None)])
        self.assertEqual(changed_package_keys(old, new), {("b", "2.0.0")})

    def test_unrelated_package_untouched_is_not_reported(self) -> None:
        old = lock([("a", "1.0.0", None, None), ("b", "1.0.0", None, None)])
        new = lock([("a", "1.0.1", None, None), ("b", "1.0.0", None, None)])
        self.assertEqual(changed_package_keys(old, new), {("a", "1.0.0"), ("a", "1.0.1")})


class FullDependencyClosureTests(unittest.TestCase):
    def test_transitive_external_dependency_is_reachable(self) -> None:
        md = metadata(
            {"top": ("crates/top", ["mid"])},
            external={"mid": "1.0.0", "leaf": "2.0.0"},
        )
        # wire mid -> leaf manually since metadata() gives external nodes no deps
        for node in md["resolve"]["nodes"]:
            if node["id"] == "mid":
                node["deps"] = [{"name": "leaf", "pkg": "leaf"}]
        closures = full_dependency_closure(md)
        self.assertEqual(closures["top"], {"mid", "leaf"})

    def test_leaf_with_no_deps_has_empty_closure(self) -> None:
        md = metadata({}, external={"leaf": "1.0.0"})
        closures = full_dependency_closure(md)
        self.assertEqual(closures["leaf"], set())

    def test_cycle_reaches_external_dependency_from_every_member(self) -> None:
        md = metadata(
            {"a": ("crates/a", ["b", "external"]), "b": ("crates/b", ["a"])},
            external={"external": "1.0.0"},
        )
        closures = full_dependency_closure(md)
        self.assertIn("external", closures["a"])
        self.assertIn("external", closures["b"])


class AffectedByLockfileChangesTests(unittest.TestCase):
    def test_direct_dependent_of_a_bumped_external_crate_is_affected(self) -> None:
        md = metadata({"a": ("crates/a", ["serde"])}, external={"serde": "1.0.1"})
        affected = affected_by_lockfile_changes(md, {("serde", "1.0.0"), ("serde", "1.0.1")})
        self.assertEqual(affected, ["a"])

    def test_app_only_dependency_update_stays_narrow(self) -> None:
        md = metadata(
            {
                "rusty_fair_play": ("crates/apps/rusty_fair_play", ["web_dep"]),
                "unrelated": ("crates/apps/unrelated", []),
            },
            external={"web_dep": "2.0.0"},
        )
        self.assertEqual(
            affected_by_lockfile_changes(md, {("web_dep", "1.0.0"), ("web_dep", "2.0.0")} ),
            ["rusty_fair_play"],
        )

    def test_shared_dependency_selects_all_workspace_consumers(self) -> None:
        md = metadata(
            {
                "rusty_fair_play": ("crates/apps/rusty_fair_play", ["shared"]),
                "rusty_tick": ("crates/apps/rusty_tick", ["shared"]),
            },
            external={"shared": "2.0.0"},
        )
        self.assertEqual(
            affected_by_lockfile_changes(md, {("shared", "1.0.0"), ("shared", "2.0.0")} ),
            ["rusty_fair_play", "rusty_tick"],
        )

    def test_removed_dependency_missing_from_head_metadata_falls_back_to_all(self) -> None:
        md = metadata(
            {"rusty_fair_play": ("crates/apps/rusty_fair_play", []), "rusty_tick": ("crates/apps/rusty_tick", [])},
            external={},
        )
        self.assertEqual(
            affected_by_lockfile_changes(md, {("removed_dependency", "1.0.0")} ),
            ["rusty_fair_play", "rusty_tick"],
        )

    def test_mixed_known_update_and_unknown_removal_is_not_narrowed(self) -> None:
        md = metadata(
            {"rusty_fair_play": ("crates/apps/rusty_fair_play", ["known"])},
            external={"known": "2.0.0"},
        )
        changed = {("known", "1.0.0"), ("known", "2.0.0"), ("unknown_removed", "1.0.0")}
        self.assertIsNone(reliable_lockfile_impact(md, changed))
        self.assertEqual(affected_by_lockfile_changes(md, changed), ["rusty_fair_play"])

    def test_transitive_dependent_through_another_workspace_crate_is_affected(self) -> None:
        md = metadata(
            {"a": ("crates/a", ["b"]), "b": ("crates/b", ["serde"])},
            external={"serde": "1.0.1"},
        )
        affected = affected_by_lockfile_changes(md, {("serde", "1.0.1")})
        self.assertEqual(affected, ["a", "b"])

    def test_unrelated_workspace_crate_is_not_affected(self) -> None:
        md = metadata(
            {"a": ("crates/a", ["serde"]), "unrelated": ("crates/unrelated", [])},
            external={"serde": "1.0.1"},
        )
        affected = affected_by_lockfile_changes(md, {("serde", "1.0.1")})
        self.assertEqual(affected, ["a"])

    def test_no_changed_keys_affects_nothing(self) -> None:
        md = metadata({"a": ("crates/a", ["serde"])}, external={"serde": "1.0.0"})
        self.assertEqual(affected_by_lockfile_changes(md, set()), [])

    def test_a_changed_key_absent_from_metadata_falls_back_to_every_crate(self) -> None:
        # Simulates a mismatched/stale metadata snapshot -- never silently
        # narrow past something we can't actually place in the graph.
        md = metadata(
            {"a": ("crates/a", []), "b": ("crates/b", [])},
            external={},
        )
        affected = affected_by_lockfile_changes(md, {("unknown_pkg", "9.9.9")})
        self.assertEqual(affected, ["a", "b"])
        self.assertIsNone(reliable_lockfile_impact(md, {("unknown_pkg", "9.9.9")}))

    def test_a_workspace_members_own_version_pin_changing_counts_directly(self) -> None:
        md = metadata({"a": ("crates/a", []), "b": ("crates/b", ["a"])}, external={})
        # "a" itself is in workspace_members with version "0.0.0" per the
        # metadata() helper; simulate its own key changing.
        affected = affected_by_lockfile_changes(md, {("a", "0.0.0")})
        self.assertEqual(affected, ["a", "b"])

    def test_cli_narrows_valid_input_and_rejects_malformed_or_mixed_unknown_input(self) -> None:
        md = metadata(
            {"rusty_fair_play": ("crates/apps/rusty_fair_play", ["known"])},
            external={"known": "2.0.0"},
        )
        old = lock([("known", "1.0.0", None, None)])
        new = lock([("known", "2.0.0", None, None)])
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            metadata_path = root / "metadata.json"
            old_path = root / "old.lock"
            new_path = root / "new.lock"
            metadata_path.write_text(json.dumps(md), encoding="utf-8")
            old_path.write_text(old, encoding="utf-8")
            new_path.write_text(new, encoding="utf-8")
            valid = self._run_cli(metadata_path, old_path, new_path)
            self.assertEqual(valid.returncode, 0)
            self.assertEqual(valid.stdout, "rusty_fair_play\n")

            old_path.write_text(old + lock([("unknown", "1.0.0", None, None)]), encoding="utf-8")
            mixed = self._run_cli(metadata_path, old_path, new_path)
            self.assertNotEqual(mixed.returncode, 0)

            new_path.write_text("not = [valid", encoding="utf-8")
            malformed = self._run_cli(metadata_path, old_path, new_path)
            self.assertNotEqual(malformed.returncode, 0)

    @staticmethod
    def _run_cli(metadata_path: Path, old_path: Path, new_path: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(SCRIPT), str(metadata_path), str(old_path), str(new_path)],
            capture_output=True,
            text=True,
            check=False,
        )


if __name__ == "__main__":
    unittest.main()

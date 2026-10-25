"""Unit tests for cargo_toml_diff.py against synthetic Cargo.toml text.

Run from the repo root:
    python3 -m unittest discover -s .github/scripts -p 'test_*.py'
"""
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from cargo_toml_diff import is_safe_pure_addition, member_dependencies_unchanged

SCRIPT = Path(__file__).with_name("cargo_toml_diff.py")

BASE = """\
[workspace]
resolver = "2"
members = [
    "crates/rusty_a",
    "crates/rusty_b",
]

[workspace.package]
edition = "2021"
"""


class IsSafePureAdditionTests(unittest.TestCase):
    def test_identical_text_is_safe(self) -> None:
        self.assertTrue(is_safe_pure_addition(BASE, BASE))

    def test_adding_new_members_is_safe(self) -> None:
        new = BASE.replace(
            '    "crates/rusty_b",\n',
            '    "crates/rusty_b",\n    "crates/rusty_hister/crates/rusty-hister-core",\n'
            '    "crates/rusty_hister/crates/rusty-hister-model",\n',
        )
        self.assertTrue(is_safe_pure_addition(BASE, new))

    def test_reordering_existing_members_is_safe(self) -> None:
        # Order within `members` isn't semantically meaningful to Cargo;
        # only membership matters.
        new = BASE.replace(
            '    "crates/rusty_a",\n    "crates/rusty_b",\n',
            '    "crates/rusty_b",\n    "crates/rusty_a",\n',
        )
        self.assertTrue(is_safe_pure_addition(BASE, new))

    def test_removing_a_member_is_unsafe(self) -> None:
        new = BASE.replace('    "crates/rusty_b",\n', "")
        self.assertFalse(is_safe_pure_addition(BASE, new))

    def test_renaming_a_member_is_unsafe(self) -> None:
        # A rename is a removal + addition; the removal makes it unsafe
        # even though a member was also added.
        new = BASE.replace('"crates/rusty_b"', '"crates/rusty_b_renamed"')
        self.assertFalse(is_safe_pure_addition(BASE, new))

    def test_adding_workspace_dependencies_is_safe(self) -> None:
        # Inert until a member references it; that member's manifest is selected by path.
        new = BASE + '\n[workspace.dependencies]\nserde = "1.0"\n'
        self.assertTrue(is_safe_pure_addition(BASE, new))

    def test_adding_a_member_and_its_workspace_dependency_is_safe(self) -> None:
        # The #1030 shape: a new crate plus the path entry its siblings will use.
        old = BASE + '\n[workspace.dependencies]\nrusty_a = { path = "crates/rusty_a" }\n'
        new = old.replace(
            '    "crates/rusty_b",\n', '    "crates/rusty_b",\n    "crates/rusty_c",\n'
        ) + 'rusty_c = { path = "crates/rusty_c" }\n'
        self.assertTrue(is_safe_pure_addition(old, new))

    def test_changing_a_workspace_dependency_is_unsafe(self) -> None:
        old = BASE + '\n[workspace.dependencies]\nserde = "1.0"\n'
        self.assertFalse(is_safe_pure_addition(old, old.replace('"1.0"', '"1.1"')))

    def test_removing_a_workspace_dependency_is_unsafe(self) -> None:
        old = BASE + '\n[workspace.dependencies]\nserde = "1.0"\nlog = "0.4"\n'
        self.assertFalse(is_safe_pure_addition(old, old.replace('log = "0.4"\n', "")))

    def test_workspace_package_edit_is_unsafe(self) -> None:
        new = BASE.replace('edition = "2021"', 'edition = "2024"')
        self.assertFalse(is_safe_pure_addition(BASE, new))

    def test_exclude_addition_is_unsafe(self) -> None:
        new = BASE.replace(
            'members = [',
            'exclude = ["crates/some_fuzz_target"]\nmembers = [',
        )
        self.assertFalse(is_safe_pure_addition(BASE, new))

    def test_profile_table_change_is_unsafe(self) -> None:
        new = BASE + '\n[profile.release]\nlto = true\n'
        self.assertFalse(is_safe_pure_addition(BASE, new))

    def test_unparsable_new_toml_is_unsafe(self) -> None:
        self.assertFalse(is_safe_pure_addition(BASE, "not valid toml [[["))

    def test_unparsable_old_toml_is_unsafe(self) -> None:
        self.assertFalse(is_safe_pure_addition("not valid toml [[[", BASE))


MEMBER = """\
[package]
name = "a"
version = "0.1.0"

[dependencies]
shared = { workspace = true }

[target.'cfg(windows)'.dependencies]
shared = { workspace = true, features = ["mode"] }
"""


class MemberDependenciesUnchangedTests(unittest.TestCase):
    def test_package_metadata_edit_is_local(self) -> None:
        new = MEMBER.replace('version = "0.1.0"', 'version = "0.2.0"\nedition = "2024"')
        self.assertTrue(member_dependencies_unchanged(MEMBER, new))

    def test_a_feature_moved_to_another_target_is_a_change(self) -> None:
        # The review's case: the all-target feature union is unchanged, Linux's build is not.
        new = MEMBER.replace("cfg(windows)", "cfg(unix)")
        self.assertFalse(member_dependencies_unchanged(MEMBER, new))

    def test_a_feature_moved_between_dependency_kinds_is_a_change(self) -> None:
        old = '[package]\nname = "a"\n[build-dependencies]\nshared = { version = "1", features = ["mode"] }\n'
        new = old.replace("build-dependencies", "dependencies")
        self.assertFalse(member_dependencies_unchanged(old, new))

    def test_an_added_dependency_or_feature_is_a_change(self) -> None:
        self.assertFalse(member_dependencies_unchanged(MEMBER, MEMBER + 'gone = "0.1"\n'))
        self.assertFalse(
            member_dependencies_unchanged(MEMBER, MEMBER + '\n[features]\nfast = ["shared/mode"]\n')
        )

    def test_unparsable_manifest_is_a_change(self) -> None:
        self.assertFalse(member_dependencies_unchanged(MEMBER, "not valid toml [[["))


class MemberDependencyEditsCliTests(unittest.TestCase):
    def test_lists_only_members_whose_dependencies_changed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("a", "b", "outside"):
                (root / name).mkdir()
                (root / name / "Cargo.toml").write_text(MEMBER.replace('"a"', f'"{name}"'))
            self._git(root, "init", "-q")
            self._git(root, "add", ".")
            self._git(root, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "base")
            base = self._git(root, "rev-parse", "HEAD").strip()
            (root / "a" / "Cargo.toml").write_text(MEMBER.replace("cfg(windows)", "cfg(unix)"))
            (root / "b" / "Cargo.toml").write_text(MEMBER.replace('"a"', '"b"') + "[lints]\n")
            (root / "outside" / "Cargo.toml").write_text(MEMBER + 'gone = "0.1"\n')
            (root / "c").mkdir()
            (root / "c" / "Cargo.toml").write_text(MEMBER.replace('"a"', '"c"'))
            metadata = {
                "workspace_root": str(root),
                "workspace_members": ["a", "b", "c"],
                "packages": [
                    {"id": name, "manifest_path": str(root / name / "Cargo.toml")}
                    for name in ("a", "b", "c", "outside")
                ],
            }
            (root / "metadata.json").write_text(json.dumps(metadata))
            changed = "a/Cargo.toml\nb/Cargo.toml\nc/Cargo.toml\noutside/Cargo.toml\nCargo.toml\n"
            result = subprocess.run(
                [sys.executable, str(SCRIPT), "--members", str(root / "metadata.json"), base],
                input=changed, capture_output=True, text=True, cwd=root, check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            # a: feature moved to another target; c: new member; b: lints only; outside: not a member.
            self.assertEqual(result.stdout, "a/Cargo.toml c/Cargo.toml\n")

    @staticmethod
    def _git(root: Path, *args: str) -> str:
        return subprocess.run(
            ["git", *args], cwd=root, capture_output=True, text=True, check=True
        ).stdout


if __name__ == "__main__":
    unittest.main()

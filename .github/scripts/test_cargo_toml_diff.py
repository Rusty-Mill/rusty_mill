"""Unit tests for cargo_toml_diff.py against synthetic Cargo.toml text.

Run from the repo root:
    python3 -m unittest discover -s .github/scripts -p 'test_*.py'
"""
import unittest

from cargo_toml_diff import is_safe_pure_addition

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


if __name__ == "__main__":
    unittest.main()

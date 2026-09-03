"""Unit tests for check_workspace_deps.py against synthetic cargo metadata.

Run from the repo root:
    python3 -m unittest discover -s .github/scripts -p 'test_*.py'
"""
import unittest

from check_workspace_deps import git_shadowed_workspace_members


def metadata(workspace: list[str], external: list[tuple[str, str, str | None]]) -> dict:
    """Build the subset of `cargo metadata` the checker reads.

    `workspace` lists workspace member package names (id == name, source
    omitted, matching a path-resolved member). `external` lists
    (id, name, source) tuples for non-workspace resolved packages.
    """
    packages = [{"id": name, "name": name} for name in workspace]
    packages += [
        {"id": pkg_id, "name": name, **({"source": source} if source else {})}
        for pkg_id, name, source in external
    ]
    return {
        "workspace_members": list(workspace),
        "packages": packages,
    }


class GitShadowedWorkspaceMembersTests(unittest.TestCase):
    def test_no_external_packages_is_clean(self) -> None:
        md = metadata(["rusty_simd"], [])
        self.assertEqual(git_shadowed_workspace_members(md), [])

    def test_registry_dependency_with_no_name_collision_is_clean(self) -> None:
        md = metadata(["rusty_simd"], [("serde 1.0.0", "serde", "registry+https://github.com/rust-lang/crates.io-index")])
        self.assertEqual(git_shadowed_workspace_members(md), [])

    def test_git_dependency_with_no_name_collision_is_clean(self) -> None:
        md = metadata(["rusty_simd"], [("other#abc123", "some_other_crate", "git+https://github.com/example/other")])
        self.assertEqual(git_shadowed_workspace_members(md), [])

    def test_git_dependency_shadowing_a_workspace_member_is_flagged(self) -> None:
        md = metadata(
            ["rusty_simd"],
            [("rusty_simd#38d3fae", "rusty_simd", "git+https://github.com/baileyrd/rusty_simd?rev=38d3fae")],
        )
        violations = git_shadowed_workspace_members(md)
        self.assertEqual(len(violations), 1)
        self.assertIn("rusty_simd", violations[0])

    def test_registry_dependency_shadowing_a_workspace_member_is_not_flagged(self) -> None:
        # A same-named published crate on crates.io isn't the git-divergence
        # failure mode this check exists for; it would only arise from an
        # explicit `version = "..."` override, which is its own, separate
        # decision to review by hand, not a silent divergence risk.
        md = metadata(
            ["rusty_simd"],
            [("rusty_simd 0.1.0", "rusty_simd", "registry+https://github.com/rust-lang/crates.io-index")],
        )
        self.assertEqual(git_shadowed_workspace_members(md), [])

    def test_multiple_violations_are_sorted(self) -> None:
        md = metadata(
            ["rusty_lsp", "rusty_simd"],
            [
                ("rusty_simd#38d3fae", "rusty_simd", "git+https://github.com/baileyrd/rusty_simd"),
                ("rusty_lsp#3bdbdb4", "rusty_lsp", "git+https://github.com/baileyrd/rusty_lsp"),
            ],
        )
        violations = git_shadowed_workspace_members(md)
        self.assertEqual(len(violations), 2)
        self.assertEqual(violations, sorted(violations))


if __name__ == "__main__":
    unittest.main()

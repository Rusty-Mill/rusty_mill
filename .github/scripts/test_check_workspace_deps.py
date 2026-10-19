"""Unit tests for check_workspace_deps.py against synthetic cargo metadata.

Run from the repo root:
    python3 -m unittest discover -s .github/scripts -p 'test_*.py'
"""
import unittest

from check_workspace_deps import (
    PINNED_EXTERNAL_DEPENDENCIES,
    TRACING_BASELINE,
    TRACING_FACADE,
    git_shadowed_workspace_members,
    pinned_dependency_violations,
    tracing_baseline_violations,
)


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


REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"


def crate_metadata(name: str, dependencies: list[dict]) -> dict:
    """The subset of `cargo metadata` the pinned-dependency check reads."""
    return {
        "workspace_members": [name],
        "packages": [{"id": name, "name": name, "dependencies": dependencies}],
    }


def dep(name: str, kind: str | None = None, source: str | None = REGISTRY) -> dict:
    return {"name": name, "kind": kind, **({"source": source} if source else {})}


class PinnedDependencyTests(unittest.TestCase):
    PINNED = {"c": {"axum": "why", "tracing": "why"}}

    def check(self, dependencies: list[dict]) -> list[str]:
        return pinned_dependency_violations(crate_metadata("c", dependencies), self.PINNED)

    def test_exactly_the_allow_list_is_clean(self) -> None:
        self.assertEqual(self.check([dep("axum"), dep("tracing")]), [])

    def test_a_new_external_dependency_is_flagged_with_its_name(self) -> None:
        violations = self.check([dep("axum"), dep("tracing"), dep("opentelemetry")])
        self.assertEqual(len(violations), 1)
        self.assertIn("`opentelemetry`", violations[0])
        self.assertIn("not on its allow-list", violations[0])

    def test_a_new_build_dependency_is_flagged_too(self) -> None:
        violations = self.check([dep("axum"), dep("tracing"), dep("cc", kind="build")])
        self.assertEqual(len(violations), 1)
        self.assertIn("`cc`", violations[0])

    def test_dev_dependencies_are_exempt(self) -> None:
        self.assertEqual(self.check([dep("axum"), dep("tracing"), dep("reqwest", kind="dev")]), [])

    def test_workspace_path_dependencies_are_not_external(self) -> None:
        self.assertEqual(
            self.check([dep("axum"), dep("tracing"), dep("rusty_json", source=None)]), []
        )

    def test_a_git_dependency_is_external(self) -> None:
        violations = self.check(
            [dep("axum"), dep("tracing"), dep("fork", source="git+https://example.invalid/fork")]
        )
        self.assertEqual(len(violations), 1)

    def test_an_entry_the_crate_no_longer_uses_must_be_removed(self) -> None:
        violations = self.check([dep("tracing")])
        self.assertEqual(len(violations), 1)
        self.assertIn("`axum`", violations[0])
        self.assertIn("no longer a dependency", violations[0])

    def test_an_optional_dependency_still_counts(self) -> None:
        optional = {**dep("opentelemetry"), "optional": True}
        self.assertEqual(len(self.check([dep("axum"), dep("tracing"), optional])), 1)

    def test_a_crate_that_is_not_pinned_is_ignored(self) -> None:
        md = crate_metadata("other", [dep("anything")])
        self.assertEqual(pinned_dependency_violations(md, self.PINNED), [])

    def test_a_same_named_package_outside_the_workspace_is_ignored(self) -> None:
        md = {
            "workspace_members": [],
            "packages": [{"id": "c 1.0 (registry)", "name": "c", "dependencies": [dep("x")]}],
        }
        self.assertEqual(pinned_dependency_violations(md, self.PINNED), [])

    def test_every_pinned_entry_states_a_reason(self) -> None:
        for crate, allowed in PINNED_EXTERNAL_DEPENDENCIES.items():
            for name, reason in allowed.items():
                self.assertTrue(reason.strip(), f"{crate}: `{name}` has no reason")


class TracingBaselineTests(unittest.TestCase):
    BASELINE = {"old": frozenset({"tracing", "tracing-subscriber"})}

    def check(self, name: str, dependencies: list[dict]) -> list[str]:
        return tracing_baseline_violations(
            crate_metadata(name, dependencies), self.BASELINE, "facade"
        )

    def test_exactly_the_baseline_is_clean(self) -> None:
        self.assertEqual(self.check("old", [dep("tracing"), dep("tracing-subscriber")]), [])

    def test_a_crate_off_the_baseline_that_adds_tracing_is_told_to_use_the_facade(self) -> None:
        violations = self.check("new", [dep("tracing")])
        self.assertEqual(len(violations), 1)
        self.assertIn("new: depends on `tracing` directly", violations[0])
        self.assertIn("use `facade`", violations[0])

    def test_any_tracing_crate_counts_not_only_the_two_in_use(self) -> None:
        self.assertEqual(len(self.check("new", [dep("tracing-appender")])), 1)
        self.assertEqual(len(self.check("new", [dep("tracing-opentelemetry")])), 1)

    def test_a_name_that_only_starts_with_tracing_is_not_a_tracing_crate(self) -> None:
        self.assertEqual(self.check("new", [dep("tracingly"), dep("tracing_core_shim")]), [])

    def test_every_dependency_kind_and_source_counts(self) -> None:
        for kind in (None, "dev", "build"):
            self.assertEqual(len(self.check("new", [dep("tracing", kind=kind)])), 1, kind)
        self.assertEqual(len(self.check("new", [dep("tracing", source=None)])), 1)

    def test_a_baseline_crate_gaining_another_tracing_crate_is_flagged(self) -> None:
        violations = self.check("old", [dep("tracing"), dep("tracing-subscriber"), dep("tracing-log")])
        self.assertEqual(len(violations), 1)
        self.assertIn("`tracing-log`", violations[0])

    def test_a_stale_entry_is_flagged_so_the_baseline_only_shrinks(self) -> None:
        violations = self.check("old", [dep("tracing")])
        self.assertEqual(len(violations), 1)
        self.assertIn("`tracing-subscriber` is on TRACING_BASELINE", violations[0])
        self.assertIn("delete it from the baseline", violations[0])

    def test_a_migrated_crate_must_leave_the_baseline(self) -> None:
        violations = self.check("old", [dep("facade", source=None)])
        self.assertEqual(len(violations), 2)

    def test_the_facade_itself_is_exempt(self) -> None:
        self.assertEqual(self.check("facade", [dep("tracing"), dep("tracing-subscriber")]), [])

    def test_a_package_outside_the_workspace_is_ignored(self) -> None:
        outside = {
            "workspace_members": [],
            "packages": [{"id": "x", "name": "new", "dependencies": [dep("tracing")]}],
        }
        self.assertEqual(tracing_baseline_violations(outside, self.BASELINE, "facade"), [])

    def test_the_real_baseline_lists_only_tracing_crates_and_not_the_facade(self) -> None:
        self.assertNotIn(TRACING_FACADE, TRACING_BASELINE)
        for crate, names in TRACING_BASELINE.items():
            self.assertTrue(names, crate)
            for name in names:
                self.assertTrue(name == "tracing" or name.startswith("tracing-"), (crate, name))


if __name__ == "__main__":
    unittest.main()

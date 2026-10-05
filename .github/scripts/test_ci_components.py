"""Coverage and lane stability for independent and shared application changes."""

import unittest

from affected_crates import affected_packages
from ci_components import component_scopes
from test_affected_crates import metadata


class ComponentScopeTests(unittest.TestCase):
    def setUp(self) -> None:
        self.metadata = metadata({
            "shared": ("crates/libs/net/shared", []),
            "a-core": ("crates/apps/a/crates/core", ["shared"]),
            "a": ("crates/apps/a", ["a-core"]),
            "b": ("crates/apps/b", ["shared"]),
            "unrelated": ("crates/apps/unrelated", []),
        })

    def scope(self, *paths: str) -> list[dict[str, str]]:
        selected = affected_packages(self.metadata, paths)
        return component_scopes(self.metadata, selected)

    def test_app_a_then_app_b_have_independent_lanes(self) -> None:
        self.assertEqual(self.scope("crates/apps/a/src/lib.rs"), [
            {"component": "apps--a", "packages": "a"},
        ])
        self.assertEqual(self.scope("crates/apps/b/src/lib.rs"), [
            {"component": "apps--b", "packages": "b"},
        ])

    def test_same_app_pushes_share_a_lane_even_when_package_sets_differ(self) -> None:
        first = self.scope("crates/apps/a/crates/core/src/lib.rs")
        second = self.scope("crates/apps/a/src/lib.rs")
        self.assertEqual(first[0]["component"], second[0]["component"])
        self.assertEqual(first[0]["packages"], "a a-core")
        self.assertEqual(second[0]["packages"], "a")
        # The workflow queues both exact-SHA jobs; the second set is NOT a
        # substitute for the first (it omits a-core's own tests).

    def test_shared_library_fans_out_to_all_transitive_consumers(self) -> None:
        scopes = self.scope("crates/libs/net/shared/src/lib.rs")
        self.assertEqual(scopes, [
            {"component": "apps--a", "packages": "a a-core"},
            {"component": "apps--b", "packages": "b"},
            {"component": "libs--net", "packages": "shared"},
        ])

    def test_cross_app_diff_preserves_the_union_without_duplicates(self) -> None:
        scopes = self.scope("crates/apps/a/src/lib.rs", "crates/apps/b/src/lib.rs")
        self.assertEqual([s["packages"] for s in scopes], ["a", "b"])

    def test_component_order_is_stable_and_selection_is_not_broadened(self) -> None:
        self.assertEqual(component_scopes(self.metadata, ["b", "a", "b"]),
                         component_scopes(self.metadata, ["a", "b"]))

    def test_empty_and_unknown_selection(self) -> None:
        self.assertEqual(component_scopes(self.metadata, []), [])
        with self.assertRaisesRegex(ValueError, "absent from workspace"):
            component_scopes(self.metadata, ["missing"])

    def test_unusual_directory_names_do_not_enter_shell_log_paths(self) -> None:
        md = metadata({"safe-package": ("crates/apps/app $(unsafe)", [])})
        self.assertEqual(component_scopes(md, ["safe-package"]), [
            {"component": "workspace-other", "packages": "safe-package"},
        ])

    def test_large_matrix_falls_back_without_dropping_packages(self) -> None:
        md = metadata({f"app{i}": (f"crates/apps/app{i}", []) for i in range(129)})
        scopes = component_scopes(md, md["workspace_members"])
        self.assertEqual(len(scopes), 1)
        self.assertEqual(scopes[0]["component"], "scoped-workspace")
        self.assertEqual(set(scopes[0]["packages"].split()), set(md["workspace_members"]))


if __name__ == "__main__":
    unittest.main()

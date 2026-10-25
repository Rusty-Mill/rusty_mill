"""Coverage and lane stability for independent and shared application changes."""

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from affected_crates import affected_packages
from ci_components import component_scopes, covers_most_of_workspace
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


class CoversMostOfWorkspaceTests(unittest.TestCase):
    # Same workspace, four components: libs--net, apps--a, apps--b, apps--unrelated.
    setUp = ComponentScopeTests.setUp
    scope = ComponentScopeTests.scope

    def test_one_app_is_scoped(self) -> None:
        self.assertFalse(covers_most_of_workspace(self.metadata, self.scope("crates/apps/a/src/lib.rs")))

    def test_exactly_half_stays_scoped(self) -> None:
        scopes = self.scope("crates/apps/a/src/lib.rs", "crates/apps/b/src/lib.rs")
        self.assertEqual(len(scopes), 2)
        self.assertFalse(covers_most_of_workspace(self.metadata, scopes))

    def test_a_shared_library_reaching_most_components_prefers_the_full_sweep(self) -> None:
        scopes = self.scope("crates/libs/net/shared/src/lib.rs")
        self.assertEqual(len(scopes), 3)
        self.assertTrue(covers_most_of_workspace(self.metadata, scopes))

    def test_cli_exits_3_only_with_the_flag_and_most_components(self) -> None:
        script = Path(__file__).with_name("ci_components.py")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "metadata.json"
            path.write_text(json.dumps(self.metadata), encoding="utf-8")

            def run(packages: str, *flags: str) -> subprocess.CompletedProcess[str]:
                return subprocess.run(
                    [sys.executable, str(script), str(path), "--packages", packages, *flags],
                    capture_output=True, text=True, check=False,
                )

            most = "shared a-core a b"
            self.assertEqual(run(most, "--full-if-most").returncode, 3)
            self.assertEqual(run(most).returncode, 0)
            scoped = run("a", "--full-if-most")
            self.assertEqual(scoped.returncode, 0)
            self.assertEqual(json.loads(scoped.stdout), [{"component": "apps--a", "packages": "a"}])


if __name__ == "__main__":
    unittest.main()

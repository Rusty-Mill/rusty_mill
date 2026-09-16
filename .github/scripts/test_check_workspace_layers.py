"""Test the layer policy with synthetic Cargo resolve graphs, without Cargo."""
import unittest

from check_workspace_layers import package_family, workspace_layer_violations


def metadata(packages: list[tuple[str, str, str]], edges: list[tuple[str, str]]) -> dict:
    """Build members (name, layer, family) and resolved caller/target edges."""
    return {
        "workspace_root": "/work/crates/checkout",
        "workspace_members": [name for name, _, _ in packages],
        "packages": [
            {"id": name, "name": name,
             "manifest_path": f"/work/crates/checkout/crates/{family}/crates/{name}/Cargo.toml",
             "metadata": {"rusty_mill": {"layer": layer}}}
            for name, layer, family in packages
        ],
        "resolve": {"nodes": [
            {"id": name, "deps": [
                {"name": target, "pkg": target, "dep_kinds": [{"kind": None}]}
                for caller, target in edges if caller == name
            ]}
            for name, _, _ in packages
        ]},
    }


class WorkspaceLayerTests(unittest.TestCase):
    def test_clean_workspace(self) -> None:
        self.assertEqual(workspace_layer_violations(metadata([
            ("base", "foundation", "base"), ("app", "apps", "app")
        ], [("app", "base")])), [])

    def test_missing_table_key_and_invalid_values(self) -> None:
        for value in ({}, {"rusty_mill": {}}, {"rusty_mill": {"layer": "unknown"}},
                      {"rusty_mill": {"layer": []}}, {"rusty_mill": None}, None):
            with self.subTest(value=value):
                md = metadata([("base", "foundation", "base")], [])
                md["packages"][0]["metadata"] = value
                violations = workspace_layer_violations(md)
                self.assertEqual(len(violations), 1)
                self.assertIn("base: missing or unrecognized", violations[0])

    def test_upward_edge_all_dependency_kinds(self) -> None:
        for kind in (None, "dev", "build"):
            with self.subTest(kind=kind):
                md = metadata([("base", "foundation", "base"),
                               ("os", "platform", "os")], [("base", "os")])
                md["resolve"]["nodes"][0]["deps"][0]["dep_kinds"] = [{"kind": kind}]
                self.assertEqual(workspace_layer_violations(md), [
                    "upward dependency: base (foundation) -> os (platform)"
                ])

    def test_downward_edge(self) -> None:
        md = metadata([("base", "foundation", "base"),
                       ("os", "platform", "os")], [("os", "base")])
        self.assertEqual(workspace_layer_violations(md), [])

    def test_same_family_apps_edge(self) -> None:
        md = metadata([("cli", "apps", "product"),
                       ("core", "apps", "product")], [("cli", "core")])
        self.assertEqual(workspace_layer_violations(md), [])

    def test_cross_family_apps_edge(self) -> None:
        md = metadata([("cli", "apps", "one"),
                       ("core", "apps", "two")], [("cli", "core")])
        self.assertEqual(workspace_layer_violations(md), [
            "cross-family apps dependency: cli (apps) -> core (apps)"
        ])

    def test_rusty_boot_to_rush_is_exempt(self) -> None:
        md = metadata([("rusty_boot", "tools", "rusty_boot"),
                       ("rush", "apps", "rush")], [("rusty_boot", "rush")])
        for pkg in md["packages"]:
            pkg["manifest_path"] = f"{md['workspace_root']}/crates/{pkg['name']}/Cargo.toml"
        self.assertEqual(workspace_layer_violations(md), [])

    def test_external_edges_ignored(self) -> None:
        md = metadata([("base", "foundation", "base")], [("base", "external")])
        md["packages"].append({"id": "external", "name": "external"})
        md["resolve"]["nodes"].append({"id": "external", "deps": [{"pkg": "base"}]})
        self.assertEqual(workspace_layer_violations(md), [])

    def test_windows_family_uses_workspace_relative_path(self) -> None:
        pkg = {"name": "core", "manifest_path":
               "C:\\crates\\checkout\\crates\\product\\crates\\core\\Cargo.toml"}
        self.assertEqual(package_family(pkg, "C:\\crates\\checkout"), "product")

    def test_family_skips_the_layer_directory_once_a_family_has_moved_under_it(self) -> None:
        # ADR-0003 Phase 1: crates/rustils/crates/platform-linux moved to
        # crates/platform/rustils/crates/platform-linux. The family is
        # still "rustils", not "platform" (the layer) -- collapsing every
        # crate in a layer into one fake family would silently disable the
        # cross-family apps check.
        pkg = {"name": "platform-linux", "manifest_path":
               "/work/crates/platform/rustils/crates/platform-linux/Cargo.toml"}
        self.assertEqual(package_family(pkg, "/work"), "rustils")

    def test_family_of_a_single_crate_family_moved_directly_under_its_layer(self) -> None:
        # A single-crate family (directory name == crate name) that lands
        # straight under its layer with no extra nesting, e.g.
        # crates/foundation/rpath -- family is the crate's own directory
        # name, not the layer.
        pkg = {"name": "rpath", "manifest_path": "/work/crates/foundation/rpath/Cargo.toml"}
        self.assertEqual(package_family(pkg, "/work"), "rpath")

    def test_family_before_any_move_is_unaffected_by_the_layer_skip(self) -> None:
        pkg = {"name": "rusty_boot", "manifest_path": "/work/crates/rusty_boot/Cargo.toml"}
        self.assertEqual(package_family(pkg, "/work"), "rusty_boot")


if __name__ == "__main__":
    unittest.main()

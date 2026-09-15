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


if __name__ == "__main__":
    unittest.main()

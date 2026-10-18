"""Exercise Orch's bundled analyzer against CI policy fixtures and real manifests."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

from test_check_workspace_layers import metadata

SOURCE = Path(__file__).resolve().parents[2] / "crates/apps/rusty_orch/crates/rusty_orch/src/intelligence.py"
SPEC = importlib.util.spec_from_file_location("repository_intelligence", SOURCE)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class IntelligenceTests(unittest.TestCase):
    def fixture(self, members, edges):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        md = metadata(members, edges)
        old_root = md["workspace_root"]
        md["workspace_root"] = str(root)
        for pkg in md["packages"]:
            path = root / Path(pkg["manifest_path"]).relative_to(old_root)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('[package]\nname = "' + pkg["name"] + '"\nversion = "0.1.0"\n', encoding="utf-8")
            pkg["manifest_path"] = str(path)
        return md

    def test_graph_layer_evidence_and_transitive_impact(self):
        md = self.fixture([("base", "foundation", "base"), ("app", "apps", "product"),
                           ("cli", "apps", "product")], [("base", "app"), ("cli", "base")])
        report = MODULE.analyze(md)
        self.assertEqual(report["schema_version"], 1)
        findings = report["findings"]
        self.assertEqual(len(findings), 1)
        self.assertEqual(findings[0]["rule"], "dependency-layer")
        self.assertEqual(findings[0]["affected_crates"], ["app", "base"])
        self.assertEqual(findings[0]["impact"]["crates_to_validate"], ["app", "base", "cli"])
        self.assertIn("upward dependency", findings[0]["evidence"][0]["message"])
        self.assertEqual(next(c for c in report["crates"] if c["id"] == "base")["reverse_dependencies"], ["cli"])
        self.assertEqual(report, MODULE.analyze(md))

    def test_missing_classification_and_cross_family_apps(self):
        md = self.fixture([("a", "apps", "one"), ("b", "apps", "two"),
                           ("c", "unknown", "three")], [("a", "b")])
        findings = MODULE.analyze(md)["findings"]
        self.assertEqual({f["rule"] for f in findings}, {"layer-classification", "dependency-layer"})
        self.assertTrue(any("cross-family" in f["evidence"][0]["message"] for f in findings))

    def test_duplicate_declarations_are_exact_and_ignore_inheritance_and_paths(self):
        md = self.fixture([("a", "libs", "a"), ("b", "libs", "b")], [])
        for pkg in md["packages"]:
            with Path(pkg["manifest_path"]).open("a", encoding="utf-8") as manifest:
                manifest.write('[dependencies]\nshared = "1"\nlocal = { path = "../local" }\ninherited = { workspace = true }\n')
        findings = MODULE.analyze(md)["findings"]
        self.assertEqual(len(findings), 1)
        self.assertEqual(findings[0]["evidence"][0]["dependency"], "shared")
        path = Path(md["packages"][0]["manifest_path"])
        path.write_text(path.read_text().replace('shared = "1"', 'shared = { version = "1", features = ["extra"] }'))
        self.assertEqual(MODULE.analyze(md)["findings"], [])

    def test_existing_first_party_shadow_and_pinned_checks_are_reused(self):
        md = self.fixture([("local", "foundation", "local"), ("user", "apps", "user"),
                           ("rusty-mcp", "libs", "mcp")], [])
        md["packages"].append({"id": "git-copy", "name": "local", "source": "git+https://example.test/local"})
        md["resolve"]["nodes"][1]["deps"] = [{"pkg": "git-copy", "name": "local"}]
        md["packages"][2]["dependencies"] = [{"name": "unapproved", "source": "registry+example", "kind": None}]
        findings = MODULE.analyze(md)["findings"]
        shadow = next(f for f in findings if f["rule"] == "first-party-git-shadow")
        self.assertEqual(shadow["affected_crates"], ["local", "user"])
        self.assertTrue(any(f["rule"] == "pinned-external-dependency" for f in findings))

    def test_manifest_outside_workspace_fails_before_reading_it(self):
        md = self.fixture([("a", "foundation", "a")], [])
        md["packages"][0]["manifest_path"] = "/outside/Cargo.toml"
        with self.assertRaises(ValueError):
            MODULE.analyze(md)

    def test_invalid_metadata_fails_instead_of_reporting_clean(self):
        md = self.fixture([("a", "foundation", "a")], [])
        md["resolve"] = None
        with self.assertRaisesRegex(ValueError, "complete Cargo resolve"):
            MODULE.analyze(md)

    def test_cycles_terminate_and_include_all_reverse_dependents(self):
        self.assertEqual(MODULE.closure(["a"], {"a": {"b"}, "b": {"a", "c"}, "c": set()}), ["a", "b", "c"])

    def test_allowed_graph_is_clean_and_json_serializable(self):
        md = self.fixture([("base", "foundation", "base"), ("app", "apps", "app")], [("app", "base")])
        report = MODULE.analyze(md)
        self.assertEqual(report["findings"], [])
        self.assertEqual(json.loads(json.dumps(report)), report)


if __name__ == "__main__":
    unittest.main()

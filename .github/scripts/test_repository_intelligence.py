"""Exercise Orch's bundled analyzer against CI policy fixtures and real manifests."""
import importlib.util
import json
import os
from pathlib import Path, PureWindowsPath
import subprocess
import sys
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

    def test_manifest_paths_use_portable_slashes_for_maintenance(self):
        md = self.fixture([("a", "unknown", "a")], [])
        report = MODULE.analyze(md)
        expected = "crates/a/crates/a/Cargo.toml"
        self.assertEqual(report["crates"][0]["manifest"], expected)
        self.assertEqual(report["findings"][0]["evidence"][0]["manifest"], expected)

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

    def test_git_shadow_impact_traverses_external_nodes_and_cycles(self) -> None:
        md = self.fixture([("local", "foundation", "local"), ("app", "apps", "app"),
                           ("cli", "tools", "cli"), ("unrelated", "libs", "unrelated")],
                          [("cli", "app")])
        md["packages"].append({"id": "git-copy", "name": "local", "source": "git+https://example.test/local"})
        md["resolve"]["nodes"][1]["deps"] = [{"pkg": "adapter", "name": "adapter"}]
        md["resolve"]["nodes"].extend([
            {"id": "adapter", "deps": [{"pkg": "bridge"}]},
            {"id": "bridge", "deps": [{"pkg": "adapter"}, {"pkg": "git-copy"}]},
            {"id": "git-copy", "deps": []},
        ])
        shadow, = MODULE.analyze(md)["findings"]
        self.assertEqual(shadow["affected_crates"], ["app", "cli", "local"])
        self.assertEqual(shadow["impact"]["crates_to_validate"], ["app", "cli", "local"])

    def test_finding_ids_survive_relocation_with_cargo_file_urls(self) -> None:
        reports = []
        for location in ("first checkout", "second checkout"):
            md = self.fixture([("base", "foundation", "base"), ("app", "apps", "app")], [("base", "app")])
            root = Path(md["workspace_root"])
            relocated = root / location
            relocated.mkdir()
            (root / "crates").rename(relocated / "crates")
            md["workspace_root"] = str(relocated)
            ids = {pkg["id"]: "path+" + (relocated / "crates" / pkg["name"]).as_uri() + "#0.1.0"
                   for pkg in md["packages"]}
            md["workspace_members"] = [ids[key] for key in md["workspace_members"]]
            for pkg in md["packages"]:
                pkg["id"] = ids[pkg["id"]]
                pkg["manifest_path"] = str(relocated / Path(pkg["manifest_path"]).relative_to(root))
            for node in md["resolve"]["nodes"]:
                node["id"] = ids[node["id"]]
                for dep in node["deps"]:
                    dep["pkg"] = ids[dep["pkg"]]
            reports.append(MODULE.analyze(md))
        self.assertEqual(reports[0]["findings"][0]["id"], reports[1]["findings"][0]["id"])
        self.assertNotEqual(reports[0]["findings"][0]["evidence"], reports[1]["findings"][0]["evidence"])

    def test_identity_normalizes_windows_paths_before_json_escaping(self) -> None:
        roots = [PureWindowsPath("C:/first checkout"), PureWindowsPath("D:/second checkout")]
        identities = [MODULE.finding_identity("rule", [{"path": str(root / "crate"),
                       "id": "path+" + root.as_uri() + "/crate#0.1.0"}], root) for root in roots]
        self.assertEqual(*identities)

    @unittest.skipUnless(sys.platform == "win32", "Windows extended-length paths")
    def test_mixed_windows_path_spellings_are_equivalent(self) -> None:
        md = self.fixture([("base", "foundation", "base"), ("app", "apps", "app")], [("base", "app")])
        expected = MODULE.analyze(md)
        md["workspace_root"] = "\\\\?\\" + md["workspace_root"]
        md["packages"][0]["manifest_path"] = "\\\\?\\" + md["packages"][0]["manifest_path"]
        self.assertEqual(MODULE.analyze(md), expected)

    def test_real_cargo_git_shadow_through_non_workspace_adapter(self) -> None:
        md = self.fixture([("local", "foundation", "local"), ("app", "apps", "app"),
                           ("cli", "tools", "cli")], [])
        root = Path(md["workspace_root"])

        def write(relative: str, contents: str) -> None:
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(contents, encoding="utf-8")

        def run(*argv: str, cwd: Path = root) -> str:
            result = subprocess.run(argv, cwd=cwd, capture_output=True, text=True, timeout=90,
                                    env=dict(os.environ, CARGO_HOME=str(root / "cargo-cache")))
            self.assertEqual(result.returncode, 0, result.stderr)
            return result.stdout

        members = [str(Path(pkg["manifest_path"]).parent.relative_to(root)).replace("\\", "/")
                   for pkg in md["packages"]]
        write("Cargo.toml", '[workspace]\nresolver = "2"\nmembers = ' + json.dumps(members)
              + '\nexclude = ["vendor/adapter", "vendor/shadow"]\n')
        for pkg in md["packages"]:
            directory = Path(pkg["manifest_path"]).parent
            (directory / "src").mkdir()
            (directory / "src/lib.rs").write_text("", encoding="utf-8")
            with Path(pkg["manifest_path"]).open("a", encoding="utf-8") as manifest:
                manifest.write('[package.metadata.rusty_mill]\nlayer = "'
                               + pkg["metadata"]["rusty_mill"]["layer"] + '"\n')
        write("vendor/shadow/Cargo.toml", '[package]\nname = "local"\nversion = "0.1.0"\n')
        write("vendor/shadow/src/lib.rs", "")
        origin = root / "vendor/shadow"
        run("git", "init", cwd=origin)
        run("git", "add", ".", cwd=origin)
        run("git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.test",
            "-c", "commit.gpgsign=false", "commit", "-m", "fixture", cwd=origin)
        write("vendor/adapter/Cargo.toml", '[package]\nname = "adapter"\nversion = "0.1.0"\n'
              + '[dependencies]\nlocal = { git = "' + origin.as_uri() + '" }\n')
        write("vendor/adapter/src/lib.rs", "")
        for index, dependency in [(1, 'adapter = { path = "' + (root / "vendor/adapter").as_posix() + '" }'),
                                  (2, 'app = { path = "' + Path(md["packages"][1]["manifest_path"]).parent.as_posix() + '" }')]:
            with Path(md["packages"][index]["manifest_path"]).open("a", encoding="utf-8") as manifest:
                manifest.write("[dependencies]\n" + dependency + "\n")
        run("cargo", "generate-lockfile")  # Only the local file:// git fixture is fetched.
        resolved = json.loads(run("cargo", "metadata", "--format-version=1", "--all-features", "--frozen"))
        report = MODULE.analyze(resolved)
        shadow, = report["findings"]
        names = {pkg["id"]: pkg["name"] for pkg in resolved["packages"]}
        self.assertEqual({names[key] for key in shadow["affected_crates"]}, {"local", "app", "cli"})
        self.assertEqual(shadow["impact"]["crates_to_validate"], shadow["affected_crates"])

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

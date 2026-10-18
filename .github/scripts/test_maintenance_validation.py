"""Exercise the frozen BBP maintenance scope gate against real Git candidates."""
import pathlib
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
PROGRAM = ROOT / "crates/apps/rusty_orch/crates/rusty_orch/src/maintenance_validation.py"


class ScopeGate(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = pathlib.Path(self.temp.name)
        self.git("init", "-q")
        self.git("config", "user.name", "Test")
        self.git("config", "user.email", "test@example.invalid")
        self.write("Cargo.toml", '[workspace]\nmembers = ["crates/apps/a", "crates/apps/b"]\n')
        for name in ("a", "b"):
            self.write(f"crates/apps/{name}/Cargo.toml", f'[package]\nname = "{name}"\nversion = "0.1.0"\n[dependencies]\nserde = "1"\n')
        self.git("add", ".")
        self.git("commit", "-qm", "base")

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.repo), *args])

    def write(self, path, text):
        destination = self.repo / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(text)

    def check(self, selection):
        import json
        self.git("add", ".")
        return subprocess.run(["python3", "-I", str(PROGRAM), json.dumps(selection)], cwd=self.repo, capture_output=True)

    def layer(self):
        return {"rule": "layer-classification", "allowed_paths": ["crates/apps/a/Cargo.toml"]}

    def repair_layer(self):
        path = self.repo / "crates/apps/a/Cargo.toml"
        path.write_text(path.read_text() + '[package.metadata.rusty_mill]\nlayer = "apps"\n')

    def test_layer_repair_is_a_beneficial_metadata_only_change(self):
        self.repair_layer()
        result = self.check(self.layer())
        self.assertEqual(result.returncode, 0, result.stderr)
        path = self.repo / "crates/apps/a/Cargo.toml"
        path.write_text(path.read_text().replace('version = "0.1.0"', 'version = "0.2.0"'))
        self.assertNotEqual(self.check(self.layer()).returncode, 0)

    def test_wrong_layer_out_of_scope_and_deletion_are_refused(self):
        self.repair_layer()
        path = self.repo / "crates/apps/a/Cargo.toml"
        path.write_text(path.read_text().replace('layer = "apps"', 'layer = "libs"'))
        self.assertNotEqual(self.check(self.layer()).returncode, 0)
        self.repair_layer()
        self.write("surprise", "outside allowed paths")
        self.assertNotEqual(self.check(self.layer()).returncode, 0)
        (self.repo / "surprise").unlink()
        path.unlink()
        self.assertNotEqual(self.check(self.layer()).returncode, 0)

    def duplicate(self):
        return {"rule": "duplicate-declaration", "allowed_paths": ["Cargo.toml", "crates/apps/a/Cargo.toml", "crates/apps/b/Cargo.toml"], "evidence": [{"manifest": f"crates/apps/{name}/Cargo.toml", "table": "dependencies", "dependency": "serde", "declaration": {"version": "1"}} for name in ("a", "b")]}

    def hoist(self):
        root = self.repo / "Cargo.toml"
        root.write_text(root.read_text() + '[workspace.dependencies]\nserde = { version = "1" }\n')
        for name in ("a", "b"):
            path = self.repo / f"crates/apps/{name}/Cargo.toml"
            path.write_text(path.read_text().replace('serde = "1"', 'serde = { workspace = true }'))

    def test_confirmed_duplicate_hoist_preserves_semantics(self):
        self.hoist()
        result = self.check(self.duplicate())
        self.assertEqual(result.returncode, 0, result.stderr)
        path = self.repo / "Cargo.toml"
        path.write_text(path.read_text().replace('version = "1"', 'version = "2"'))
        self.assertNotEqual(self.check(self.duplicate()).returncode, 0)

    def test_features_and_unrelated_member_edits_are_refused(self):
        self.hoist()
        path = self.repo / "crates/apps/a/Cargo.toml"
        path.write_text(path.read_text().replace('workspace = true', 'workspace = true, features = ["derive"]'))
        self.assertNotEqual(self.check(self.duplicate()).returncode, 0)


if __name__ == "__main__":
    unittest.main()

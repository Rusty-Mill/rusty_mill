"""Test generated map content and byte-for-byte CLI verification without Cargo."""
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from generate_workspace_map import generate_workspace_map
from test_check_workspace_layers import metadata


class WorkspaceMapTests(unittest.TestCase):
    def test_columns_descriptions_and_unique_dependents(self) -> None:
        md = metadata([("base", "foundation", "base"), ("app", "apps", "app"),
                       ("tool", "tools", "tool")],
                      [("app", "base"), ("app", "base"), ("tool", "base"),
                       ("base", "base"), ("tool", "app")])
        md["packages"][0]["description"] = "First line\nsecond | <part>"
        md["packages"][1]["description"] = None
        rendered = generate_workspace_map(md)
        self.assertIn("| foundation | base | base | First line second \\| &lt;part&gt; | 2 |", rendered)
        self.assertIn("| apps | app | app |  | 1 |", rendered)
        self.assertIn("| tools | tool | tool |  | 0 |", rendered)

    def test_sort_order_and_input_order_independence(self) -> None:
        md = metadata([("z", "apps", "a"), ("b", "foundation", "z"),
                       ("z2", "foundation", "a"), ("a", "foundation", "a"),
                       ("p", "platform", "a"), ("l", "libs", "a"),
                       ("t", "tools", "a")], [])
        rendered = generate_workspace_map(md)
        rows = [line.split(" | ")[2] for line in rendered.splitlines()[16:]]
        self.assertEqual(rows, ["a", "z2", "b", "p", "l", "z", "t"])
        md["packages"].reverse()
        self.assertEqual(generate_workspace_map(md), rendered)

    def test_cli_generation_and_verify_bytes(self) -> None:
        md = metadata([("base", "foundation", "base")], [])
        md["packages"][0]["description"] = "Unicode: café"
        expected = generate_workspace_map(md).encode("utf-8")
        script = Path(__file__).with_name("generate_workspace_map.py")
        with tempfile.TemporaryDirectory() as directory:
            data = Path(directory) / "metadata.json"
            target = Path(directory) / "WORKSPACE-MAP.md"
            data.write_text(json.dumps(md), encoding="utf-8")
            command = [sys.executable, str(script), str(data)]
            result = subprocess.run(command, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, expected)
            for content in (expected, b"stale", expected.replace(b"\n", b"\r\n"), None):
                with self.subTest(content=content):
                    if content is None:
                        target.unlink()
                    else:
                        target.write_bytes(content)
                    result = subprocess.run(command + ["--verify", str(target)], capture_output=True)
                    self.assertEqual(result.returncode, 0 if content == expected else 1, result.stderr)
                    self.assertEqual(result.stdout, b"")
                    if content == expected:
                        self.assertEqual(result.stderr, b"")
                    else:
                        self.assertIn(b"is stale", result.stderr)
                        self.assertIn(b"regenerate with", result.stderr)


if __name__ == "__main__":
    unittest.main()

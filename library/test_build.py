import unittest
from pathlib import Path

import build


class ParseTests(unittest.TestCase):
    def test_frontmatter_and_list(self) -> None:
        meta, body = build.parse("---\nname: x\ntags: [a, b]\n---\nhello\n")
        self.assertEqual(meta, {"name": "x", "tags": ["a", "b"]})
        self.assertEqual(body, "hello")

    def test_missing_frontmatter_raises(self) -> None:
        with self.assertRaises(ValueError):
            build.parse("no frontmatter")

    def test_entry_requires_description(self) -> None:
        with self.assertRaises(ValueError):
            build.entry("prompt", build.ROOT / "p.md", {"name": "x"}, "b")

    def test_entry_requires_source(self) -> None:
        with self.assertRaises(ValueError):
            build.entry("prompt", build.ROOT / "p.md", {"name": "x", "description": "d"}, "b")

    def test_mod_source_excludes_tests(self) -> None:
        mod = build.load_mod(build.ROOT / "mods/backup-before-edit")
        self.assertNotIn("register.test", mod["body"])

    def test_optional_fields(self) -> None:
        meta = {"name": "x", "description": "d", "source": "s", "status": "fix coming", "featured": "true"}
        e = build.entry("prompt", build.ROOT / "p.md", meta, "b")
        self.assertEqual((e["status"], e["featured"], e["for"]), ("fix coming", True, ""))
        self.assertEqual(build.entry("skill", build.ROOT / "p.md", meta, "b")["for"], "Claude Code")

    def test_all_seeded_entries_load(self) -> None:
        kinds = {i["kind"] for i in build.collect()}
        self.assertEqual(kinds, {"prompt", "skill", "mod", "claude-md", "agents-md"})


if __name__ == "__main__":
    unittest.main()

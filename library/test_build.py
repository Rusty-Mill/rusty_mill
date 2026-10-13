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
            build.entry("prompt", "p.md", {"name": "x"}, "b")

    def test_entry_requires_source(self) -> None:
        with self.assertRaises(ValueError):
            build.entry("prompt", "p.md", {"name": "x", "description": "d"}, "b")

    def test_mod_source_excludes_tests(self) -> None:
        mod = build.load_mod(build.ROOT / "mods/backup-before-edit")
        self.assertNotIn("register.test", mod["body"])

    def test_optional_fields(self) -> None:
        meta = {"name": "x", "description": "d", "source": "s", "status": "fix coming", "featured": "true"}
        e = build.entry("prompt", "p.md", meta, "b")
        self.assertEqual((e["status"], e["featured"], e["for"]), ("fix coming", True, ""))
        self.assertEqual(build.entry("skill", "p.md", meta, "b")["for"], "Claude Code")

    def test_skill_export_round_trips(self) -> None:
        skill = next(i for i in build.collect_local() if i["kind"] == "skill")
        meta, body = build.parse(skill["export"])
        self.assertEqual((meta["name"], meta["description"], meta["source"]), (skill["name"], skill["description"], skill["source"]))
        self.assertEqual(body, skill["body"])

    def test_sections_export_body_only(self) -> None:
        for kind in ("prompt", "claude-md", "agents-md"):
            item = next(i for i in build.collect_local() if i["kind"] == kind)
            self.assertEqual(item["export"], item["body"])

    def test_duplicates_name_both_places(self) -> None:
        a = build.entry("skill", "skills/x/SKILL.md", {"name": "x", "description": "d", "source": "s"}, "b")
        b = build.entry("skill", "x", {"name": "x", "description": "d", "source": "s"}, "b", repo="o/r")
        self.assertEqual(build.duplicates([a, b]), ["skill 'x': local:skills/x/SKILL.md and o/r:x"])
        self.assertEqual(build.duplicates([a]), [])

    def test_all_seeded_entries_load(self) -> None:
        kinds = {i["kind"] for i in build.collect_local()}
        self.assertEqual(kinds, {"prompt", "skill", "mod", "claude-md", "agents-md"})


if __name__ == "__main__":
    unittest.main()

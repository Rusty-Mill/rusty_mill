import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

import build
import sources
from fmt import MAX_FILE
from sources import Source, SourceError

SKILL = "---\nname: {n}\ndescription: d\ntags: [a]\nfeatured: true\nstatus: broken\n---\nbody\n"


def write(root: Path, rel: str, text: str = "x") -> Path:
    p = root / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text)
    return p


class LoadSourcesTests(unittest.TestCase):
    def load(self, entries: object) -> list[Source]:
        with tempfile.TemporaryDirectory() as d:
            f = write(Path(d), "sources.json", json.dumps(entries))
            return sources.load_sources(f)

    def test_missing_file_means_no_sources(self) -> None:
        self.assertEqual(sources.load_sources(Path("/nonexistent/sources.json")), [])

    def test_valid_entry(self) -> None:
        self.assertEqual(self.load([{"name": "m", "repo": "o/r", "ref": "main"}]), [Source("m", "o/r", "main")])

    def test_rejects_hostile_values(self) -> None:
        bad = [
            {"name": "m", "repo": "../x/y", "ref": "main"},
            {"name": "m", "repo": "-o/r", "ref": "main"},
            {"name": "m", "repo": "o/r", "ref": "--upload-pack=evil"},
            {"name": "m", "repo": "o/r", "ref": "a..b"},
            {"name": "M x", "repo": "o/r", "ref": "main"},
            {"name": "m", "repo": "https://evil/x", "ref": "main"},
            {"name": "m", "repo": "o/r"},
        ]
        for entry in bad:
            with self.subTest(entry=entry), self.assertRaises(SourceError):
                self.load([entry])

    def test_duplicate_names_rejected(self) -> None:
        e = {"name": "m", "repo": "o/r", "ref": "main"}
        with self.assertRaises(SourceError):
            self.load([e, e])


class ScanTests(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name) / "repo"
        self.root.mkdir()
        self.addCleanup(self._tmp.cleanup)

    def kinds(self) -> dict[str, str]:
        found, _ = sources.scan(self.root)
        return {f.rel: f.kind for f in found}

    def test_finds_skills_mods_and_prompts(self) -> None:
        write(self.root, "skills/s/SKILL.md", SKILL.format(n="s"))
        write(self.root, "m/.claude-plugin/plugin.json", json.dumps({"name": "m", "description": "d"}))
        write(self.root, "m/hooks/register.ts", "code")
        write(self.root, "prompts/p.md", SKILL.format(n="p"))
        self.assertEqual(self.kinds(), {"skills/s/SKILL.md": "skill", "m": "mod", "prompts/p.md": "prompt"})

    def test_mod_body_has_readme_and_hooks_but_not_tests_or_types(self) -> None:
        write(self.root, "m/.claude-plugin/plugin.json", json.dumps({"name": "m", "description": "d", "keywords": ["k"]}))
        write(self.root, "m/README.md", "readme")
        write(self.root, "m/hooks/register.ts", "code")
        write(self.root, "m/hooks/register.test.ts", "TESTCODE")
        write(self.root, "m/tests/t.ts", "TESTCODE")
        write(self.root, "m/types/index.d.ts", "TYPES")
        (mod,), _ = sources.scan(self.root)
        self.assertEqual(mod.meta["tags"], ["k"])
        self.assertIn("readme", mod.body)
        self.assertIn("// hooks/register.ts\ncode", mod.body)
        for hidden in ("TESTCODE", "TYPES"):
            self.assertNotIn(hidden, mod.body)

    def test_source_cannot_promote_itself(self) -> None:
        write(self.root, "skills/s/SKILL.md", SKILL.format(n="s"))
        (skill,), _ = sources.scan(self.root)
        self.assertNotIn("featured", skill.meta)
        self.assertNotIn("status", skill.meta)
        self.assertEqual(skill.export, SKILL.format(n="s"))

    def test_incomplete_files_warn_instead_of_vanishing(self) -> None:
        write(self.root, "skills/a/SKILL.md", "---\nname: a\n---\nbody\n")
        write(self.root, "skills/b/SKILL.md", "no frontmatter")
        write(self.root, "m/.claude-plugin/plugin.json", "{not json")
        found, warnings = sources.scan(self.root)
        self.assertEqual(found, [])
        self.assertEqual(len(warnings), 3)

    def test_symlinks_are_never_followed(self) -> None:
        outside = write(Path(self._tmp.name), "secret/SKILL.md", SKILL.format(n="leak"))
        try:
            (self.root / "linkdir").symlink_to(outside.parent, target_is_directory=True)
            (self.root / "skills").mkdir()
            (self.root / "skills" / "SKILL.md").symlink_to(outside)
        except OSError:
            self.skipTest("symlinks unavailable")
        self.assertEqual(self.kinds(), {})

    def test_pruned_dirs_are_skipped(self) -> None:
        write(self.root, "node_modules/x/SKILL.md", SKILL.format(n="n"))
        write(self.root, ".git/x/SKILL.md", SKILL.format(n="g"))
        self.assertEqual(self.kinds(), {})

    def test_oversize_file_is_cut_with_a_marker(self) -> None:
        write(self.root, "m/.claude-plugin/plugin.json", json.dumps({"name": "m", "description": "d"}))
        write(self.root, "m/hooks/register.ts", "x" * (MAX_FILE + 10))
        (mod,), _ = sources.scan(self.root)
        self.assertIn("[truncated]", mod.body)
        self.assertLess(len(mod.body), MAX_FILE + 100)



@unittest.skipUnless(shutil.which("git"), "git not available")
class CollectSourcesTests(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.tmp = Path(self._tmp.name)
        self.cache = self.tmp / "cache"
        repo = self.cache / "pilot"
        write(repo, "skills/s/SKILL.md", SKILL.format(n="s"))
        env = {**os.environ, "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t"}
        for args in (["init", "-q"], ["add", "."], ["commit", "-q", "-m", "x"]):
            subprocess.run(["git", *args], cwd=repo, env=env, check=True, capture_output=True)
        self.sha = subprocess.run(["git", "rev-parse", "HEAD"], cwd=repo, capture_output=True, text=True).stdout.strip()
        self.file = write(self.tmp, "sources.json", json.dumps([{"name": "pilot", "repo": "o/r", "ref": self.sha}]))

    def test_entries_carry_computed_provenance_and_group(self) -> None:
        items, warnings = build.collect_sources(self.file, self.cache)
        self.assertEqual(warnings, [])
        (item,) = items
        origin = f"github.com/o/r@{self.sha[:12]}"
        self.assertEqual(item["source"], f"{origin}:skills/s/SKILL.md")
        self.assertEqual(item["group"], origin)
        self.assertEqual((item["repo"], item["path"]), ("o/r", "skills/s/SKILL.md"))
        self.assertEqual(item["tags"], ["a", "pilot"])
        self.assertEqual((item["featured"], item["status"]), (False, ""))

    def test_unsynced_source_warns(self) -> None:
        items, warnings = build.collect_sources(self.file, self.tmp / "empty-cache")
        self.assertEqual(items, [])
        self.assertEqual(warnings, ["pilot: not synced (run build.py --sync)"])


if __name__ == "__main__":
    unittest.main()

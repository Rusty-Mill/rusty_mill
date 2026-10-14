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

    def test_malformed_metadata_is_skipped_with_a_warning_not_embedded(self) -> None:
        def mod(d: str, **manifest: object) -> None:
            write(self.root, f"{d}/.claude-plugin/plugin.json", json.dumps({"name": d, "description": "d", **manifest}))

        mod("numeric-keywords", keywords=[1, 2])
        mod("keywords-not-a-list", keywords={"a": 1})
        mod("too-many-tags", keywords=[f"t{i}" for i in range(21)])
        mod("long-tag", keywords=["x" * 41])
        mod("control-in-tag", keywords=["a\x1b[31mb"])
        mod("name-is-a-list", name=["x"])
        mod("name-is-a-number", name=7)
        mod("description-is-a-list", description=["d"])
        mod("description-too-long", description="d" * 1001)
        write(self.root, "not-an-object/.claude-plugin/plugin.json", "[]")
        write(self.root, "skills/a/SKILL.md", "---\nname: [a, b]\ndescription: d\n---\nbody\n")
        write(self.root, "skills/b/SKILL.md", "---\nname: ok\ndescription: d\ntags: [1]\n---\nbody\n".replace("[1]", "[a, b]"))
        mod("fine", keywords=["k"])
        found, warnings = sources.scan(self.root)
        # `skills/b` is valid and the "fine" mod is valid; every malformed entry is reported instead.
        self.assertEqual(sorted(f.rel for f in found), ["fine", "skills/b/SKILL.md"])
        self.assertEqual(len(warnings), 11)
        for f in found:
            self.assertIsInstance(f.meta["name"], str)
            self.assertTrue(all(isinstance(t, str) for t in f.meta["tags"]))

    def test_control_characters_in_a_path_are_refused(self) -> None:
        try:
            write(self.root, "bad\nname/.claude-plugin/plugin.json", json.dumps({"name": "m", "description": "d"}))
        except OSError:
            self.skipTest("filesystem does not allow this name")
        write(self.root, "good/.claude-plugin/plugin.json", json.dumps({"name": "g", "description": "d"}))
        found, warnings = sources.scan(self.root)
        self.assertEqual([f.rel for f in found], ["good"])
        self.assertEqual(len(warnings), 1)

    def test_symlinks_are_never_followed(self) -> None:
        outside = write(Path(self._tmp.name), "secret/SKILL.md", SKILL.format(n="leak"))
        try:
            (self.root / "linkdir").symlink_to(outside.parent, target_is_directory=True)
            (self.root / "skills").mkdir()
            (self.root / "skills" / "SKILL.md").symlink_to(outside)
        except OSError:
            self.skipTest("symlinks unavailable")
        self.assertEqual(self.kinds(), {})

    def test_mod_files_reached_through_symlinks_never_reach_the_page(self) -> None:
        sentinel = "SENTINEL_OUTSIDE_CONTENT"  # synthetic: never a real secret
        outside = Path(self._tmp.name) / "outside"
        write(outside, "evil.ts", sentinel)
        write(outside, "README.md", sentinel)
        manifest = json.dumps({"name": "m", "description": "d"})
        cases = {
            "hooks dir is a symlink out": lambda m: (m / "hooks").symlink_to(outside, target_is_directory=True),
            "hook file is a symlink out": lambda m: ((m / "hooks").mkdir(), (m / "hooks" / "x.ts").symlink_to(outside / "evil.ts")),
            "README is a symlink out": lambda m: ((m / "hooks").mkdir(), (m / "README.md").symlink_to(outside / "README.md")),
            "hooks subdir is a symlink out": lambda m: ((m / "hooks").mkdir(), (m / "hooks" / "sub").symlink_to(outside, target_is_directory=True)),
        }
        for label, plant in cases.items():
            with self.subTest(label):
                mod = self.root / label.replace(" ", "_")
                write(mod, ".claude-plugin/plugin.json", manifest)
                try:
                    plant(mod)
                except OSError:
                    self.skipTest("symlinks unavailable")
                found, _ = sources.scan(self.root)
                items = [build.from_source(f, "o/r", "pilot", "0" * 40) for f in found]
                self.assertNotIn(sentinel, build.render(items))

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
        sources.record(Source("pilot", "o/r", self.sha), repo)
        self.file = write(self.tmp, "sources.json", json.dumps([{"name": "pilot", "repo": "o/r", "ref": self.sha}]))

    def configure(self, **src: str) -> Path:
        entry = {"name": "pilot", "repo": "o/r", "ref": self.sha, **src}
        return write(self.tmp, "sources.json", json.dumps([entry]))

    def assertRefused(self, file: Path, message: str) -> None:
        items, warnings = build.collect_sources(file, self.cache)
        self.assertEqual(items, [])
        self.assertEqual(len(warnings), 1)
        self.assertIn(message, warnings[0])

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

    def test_cache_for_a_different_repo_is_not_relabelled(self) -> None:
        # The reviewer's case: synced as alice/one under `pilot`, then configured as bob/two.
        self.assertRefused(self.configure(repo="bob/two"), "cache holds o/r@")

    def test_cache_at_a_different_ref_is_refused(self) -> None:
        self.assertRefused(self.configure(ref="main"), "sources.json says o/r@main")

    def test_pinned_sha_that_the_cache_is_not_at_is_refused(self) -> None:
        other = "f" * 40
        # Same recorded ref, but the config now pins another commit.
        sources.record(Source("pilot", "o/r", other), self.cache / "pilot")
        self.assertRefused(self.configure(ref=other), f"cache is at {self.sha[:12]} but sources.json pins {other[:12]}")

    def test_cache_without_a_recorded_origin_is_refused(self) -> None:
        subprocess.run(["git", "config", "--local", "--unset", "library.repo"], cwd=self.cache / "pilot", check=True)
        self.assertRefused(self.file, "no recorded origin")

    def test_plain_directory_is_not_mistaken_for_the_surrounding_repo(self) -> None:
        # The cache lives inside a larger repository, as `library/.cache` does in rusty_mill.
        env = {**os.environ, "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t"}
        outer = self.tmp / "outer"
        for args in (["init", "-q"], ["commit", "-q", "--allow-empty", "-m", "x"]):
            outer.mkdir(exist_ok=True)
            subprocess.run(["git", *args], cwd=outer, env=env, check=True, capture_output=True)
        plain = outer / "cache" / "plainsrc"
        write(plain, "skills/s/SKILL.md", SKILL.format(n="s"))
        with self.assertRaises(SourceError):
            sources.head_sha(plain)  # would otherwise report the outer repository's commit
        file = write(self.tmp, "plain.json", json.dumps([{"name": "plainsrc", "repo": "o/r", "ref": "main"}]))
        items, warnings = build.collect_sources(file, outer / "cache")
        self.assertEqual(items, [])
        self.assertIn("no recorded origin", warnings[0])

    def test_install_command_pins_the_full_commit_and_quotes_the_path(self) -> None:
        mod = self.cache / "pilot" / "mods" / "$(id) x"
        write(mod, ".claude-plugin/plugin.json", json.dumps({"name": "m", "description": "d"}))
        write(mod, "hooks/register.ts", "code")
        items, warnings = build.collect_sources(self.file, self.cache)
        (item,) = [i for i in items if i["kind"] == "mod"]
        self.assertEqual(warnings, [])
        self.assertEqual(item["rev"], self.sha)
        self.assertEqual(item["install"].count(self.sha), 2)  # once per shell
        self.assertIn("'pilot/mods/$(id) x'".replace("pilot", "r"), item["install"])

    def test_a_path_that_cannot_be_quoted_safely_gets_no_command_and_a_warning(self) -> None:
        mod = self.cache / "pilot" / 'mods' / 'say "hi"'
        write(mod, ".claude-plugin/plugin.json", json.dumps({"name": "m", "description": "d"}))
        items, warnings = build.collect_sources(self.file, self.cache)
        (item,) = [i for i in items if i["kind"] == "mod"]
        self.assertEqual(item["install"], "")
        self.assertEqual(len(warnings), 1)
        self.assertIn("no install command", warnings[0])

    def test_unsynced_source_warns(self) -> None:
        items, warnings = build.collect_sources(self.file, self.tmp / "empty-cache")
        self.assertEqual(items, [])
        self.assertEqual(warnings, ["pilot: not synced (run build.py --sync)"])


if __name__ == "__main__":
    unittest.main()

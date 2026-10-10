"""Tests for changes.py (per-PR change fragments)."""
from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import changes  # noqa: E402

REPO = Path(__file__).resolve().parents[2]
GOOD = "---\ncategory: Fixed\nchangelog: Fixed the thing.\n---\n## 2026-10-10 - Fix the thing\n\n- **Fixed:** it.\n"


class ParseTests(unittest.TestCase):
    def test_a_valid_fragment_parses(self) -> None:
        fragment = changes.parse("2026-10-10-fix-thing.md", GOOD)
        self.assertEqual((fragment.date, fragment.category), ("2026-10-10", "Fixed"))
        self.assertEqual(fragment.changelog, "Fixed the thing.")
        self.assertTrue(fragment.notes.startswith("## 2026-10-10 - Fix the thing"))

    def assert_rejected(self, name: str, text: str, expected: str) -> None:
        with self.assertRaises(ValueError) as caught:
            changes.parse(name, text)
        self.assertIn(expected, str(caught.exception))
        self.assertIn(name, str(caught.exception))

    def test_bad_names_are_rejected(self) -> None:
        for name in ("fix.md", "2026-10-10-Fix.md", "2026-10-10-.md", "2026-10-10-fix.txt"):
            with self.subTest(name=name):
                self.assert_rejected(name, GOOD, "file name must be")

    def test_front_matter_errors_are_rejected(self) -> None:
        name = "2026-10-10-x.md"
        self.assert_rejected(name, "## 2026-10-10 - x\n", "must start with")
        self.assert_rejected(name, "---\ncategory: Fixed\n", "not closed")
        self.assert_rejected(name, "---\ncategory Fixed\nchangelog: a\n---\n## 2026-10-10 - x\n", "not 'key: value'")
        self.assert_rejected(name, "---\ncategory: Fixed\n---\n## 2026-10-10 - x\n", "exactly")
        self.assert_rejected(name, GOOD.replace("Fixed\nchangelog", "Fixd\nchangelog"), "category must be")

    def test_duplicate_front_matter_keys_are_rejected(self) -> None:
        for key, line in (("category", "category: Added"), ("changelog", "changelog: Again.")):
            with self.subTest(key=key):
                text = GOOD.replace("\n---\n## ", f"\n{line}\n---\n## ", 1)
                self.assert_rejected("2026-10-10-x.md", text, f"{key!r} appears more than once")

    def test_impossible_calendar_dates_are_rejected(self) -> None:
        for date in ("2026-02-31", "2026-13-01", "2026-04-31", "2025-02-29", "2026-00-10"):
            with self.subTest(date=date):
                self.assert_rejected(f"{date}-x.md", GOOD.replace("2026-10-10", date), "is not a calendar date")

    def test_a_leap_day_is_accepted(self) -> None:
        text = GOOD.replace("2026-10-10", "2024-02-29")
        self.assertEqual(changes.parse("2024-02-29-x.md", text).date, "2024-02-29")

    def test_the_notes_heading_must_carry_the_file_date(self) -> None:
        self.assert_rejected("2026-10-11-x.md", GOOD, "must start with '## 2026-10-11 - <title>'")


class RenderTests(unittest.TestCase):
    def fragments(self) -> list[changes.Fragment]:
        old = changes.parse("2026-10-09-a.md", GOOD.replace("10-10", "10-09").replace("Fix the thing", "Old"))
        new = changes.parse("2026-10-10-b.md", GOOD.replace("Fixed\nchangelog", "Added\nchangelog").replace("Fixed the thing.", "Added a thing."))
        return [new, old]

    def test_notes_are_newest_first(self) -> None:
        out = changes.render_notes(self.fragments())
        self.assertLess(out.index("2026-10-10"), out.index("2026-10-09"))

    def test_changelog_groups_by_category_in_the_standard_order(self) -> None:
        out = changes.render_changelog(self.fragments())
        self.assertEqual(out, "### Added\n- Added a thing.\n\n### Fixed\n- Fixed the thing.\n")

    def test_load_sorts_newest_first_and_skips_the_readme(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            (directory / "README.md").write_text("not a fragment")
            (directory / "2026-10-09-a.md").write_text(GOOD.replace("10-10", "10-09"))
            (directory / "2026-10-10-b.md").write_text(GOOD)
            self.assertEqual([f.name for f in changes.load(directory)], ["2026-10-10-b.md", "2026-10-09-a.md"])


class DirectoryTests(unittest.TestCase):
    def test_a_missing_directory_is_an_error(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(ValueError) as caught:
                changes.load(Path(tmp) / "nope")
        self.assertIn("not a directory", str(caught.exception))

    def test_a_regular_file_is_not_a_directory(self) -> None:
        with tempfile.NamedTemporaryFile() as handle:
            with self.assertRaises(ValueError):
                changes.load(Path(handle.name))

    def test_an_existing_empty_directory_is_a_valid_empty_log(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(changes.load(Path(tmp)), [])

    def test_the_cli_fails_on_a_missing_path(self) -> None:
        script = str(Path(__file__).parent / "changes.py")
        with tempfile.TemporaryDirectory() as tmp:
            for command in ("check", "notes", "changelog"):
                with self.subTest(command=command):
                    result = subprocess.run(
                        [sys.executable, script, command, str(Path(tmp) / "nope")],
                        capture_output=True, text=True,
                    )
                    self.assertEqual(result.returncode, 1)
                    self.assertIn("not a directory", result.stderr)


class RepoTests(unittest.TestCase):
    def test_every_committed_fragment_is_valid(self) -> None:
        self.assertEqual(changes.main(["changes.py", "check", str(REPO / "changes")]), 0)

    def test_the_cli_fails_on_a_bad_fragment(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / "bad.md").write_text("x")
            result = subprocess.run(
                [sys.executable, str(Path(__file__).parent / "changes.py"), "check", tmp],
                capture_output=True, text=True,
            )
        self.assertEqual(result.returncode, 1)
        self.assertIn("file name must be", result.stderr)


if __name__ == "__main__":
    unittest.main()

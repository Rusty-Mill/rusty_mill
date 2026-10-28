"""Tests for libs_tag.sh against throwaway local repositories: no network, no real tags."""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("libs_tag.sh")
MONTH = "2026.10"


def git(cwd: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-c", "user.name=t", "-c", "user.email=t@example.com", *args],
        cwd=cwd, capture_output=True, text=True, check=True,
    ).stdout.strip()


class LibsTagTest(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        self.remote = root / "remote.git"
        self.clone = root / "clone"
        git(root, "init", "--quiet", "--bare", str(self.remote))
        git(root, "init", "--quiet", str(self.clone))
        git(self.clone, "commit", "--quiet", "--allow-empty", "-m", "first")
        git(self.clone, "remote", "add", "origin", str(self.remote))
        git(self.clone, "push", "--quiet", "origin", "HEAD:refs/heads/main")

    def run_script(self, month: str = MONTH) -> subprocess.CompletedProcess[str]:
        env = {**os.environ, "LIBS_TAG_MONTH": month, "GIT_CONFIG_NOSYSTEM": "1"}
        env.pop("GITHUB_STEP_SUMMARY", None)
        return subprocess.run(
            ["bash", str(SCRIPT)], cwd=self.clone, capture_output=True, text=True, env=env, check=False,
        )

    def remote_tags(self) -> dict[str, str]:
        out = git(self.remote, "for-each-ref", "--format=%(refname:short) %(objectname)", "refs/tags")
        return dict(line.split() for line in out.splitlines())

    def new_commit(self) -> str:
        git(self.clone, "commit", "--quiet", "--allow-empty", "-m", "next")
        return git(self.clone, "rev-parse", "HEAD")

    def seed_remote_tag(self, name: str) -> None:
        git(self.clone, "tag", name)
        git(self.clone, "push", "--quiet", "origin", f"refs/tags/{name}")
        git(self.clone, "tag", "-d", name)

    def test_first_tag_of_a_month_is_zero(self) -> None:
        head = git(self.clone, "rev-parse", "HEAD")
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.remote_tags(), {f"libs-{MONTH}.0": head})

    def test_counter_goes_from_nine_to_ten_numerically(self) -> None:
        for n in (2, 9):
            self.seed_remote_tag(f"libs-{MONTH}.{n}")
            self.new_commit()
        head = self.new_commit()
        self.assertEqual(self.run_script().returncode, 0)
        self.assertEqual(self.remote_tags()[f"libs-{MONTH}.10"], head)

    def test_a_new_month_starts_again_at_zero(self) -> None:
        self.seed_remote_tag(f"libs-{MONTH}.4")
        head = self.new_commit()
        self.assertEqual(self.run_script("2026.11").returncode, 0)
        self.assertEqual(self.remote_tags()["libs-2026.11.0"], head)
        self.assertNotIn(f"libs-{MONTH}.5", self.remote_tags())

    def test_a_head_that_already_has_a_tag_is_left_alone(self) -> None:
        self.assertEqual(self.run_script().returncode, 0)
        before = self.remote_tags()
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("already has", result.stdout)
        self.assertEqual(self.remote_tags(), before)

    def test_odd_suffixes_are_ignored_and_read_in_base_ten(self) -> None:
        for name in (f"libs-{MONTH}.08", f"libs-{MONTH}.x1", f"libs-{MONTH}.3rc"):
            self.seed_remote_tag(name)
            self.new_commit()
        head = self.new_commit()
        self.assertEqual(self.run_script().returncode, 0)
        self.assertEqual(self.remote_tags()[f"libs-{MONTH}.9"], head)

    def test_a_tag_that_appears_on_origin_meanwhile_rejects_the_push(self) -> None:
        # Another run wins the race: its tag lands on origin between our fetch and push.
        other = git(self.clone, "rev-parse", "HEAD")
        self.new_commit()
        hook = self.clone / ".git" / "hooks" / "pre-push"
        hook.write_text(f"#!/bin/sh\ngit --git-dir='{self.remote}' tag libs-{MONTH}.0 {other}\n", encoding="utf-8")
        hook.chmod(0o755)
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.remote_tags(), {f"libs-{MONTH}.0": other})

    def test_the_script_never_forces_or_deletes(self) -> None:
        text = SCRIPT.read_text(encoding="utf-8")
        for flag in ("--force", " -f ", "--delete", "push origin :", "+refs/"):
            with self.subTest(flag=flag):
                self.assertNotIn(flag, text)


if __name__ == "__main__":
    unittest.main()

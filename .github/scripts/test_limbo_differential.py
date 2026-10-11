"""Tests for limbo_differential.sh against a local file: no network, no cargo."""

from __future__ import annotations

import hashlib
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("limbo_differential.sh")
CORPUS = b'{"version": 1, "testcases": []}'
GOOD = hashlib.sha256(CORPUS).hexdigest()


def run(
    command: str, *, sha: str = GOOD, url: str | None = None
) -> tuple[subprocess.CompletedProcess[str], str]:
    with tempfile.TemporaryDirectory() as tmp:
        corpus = Path(tmp) / "src.json"
        corpus.write_bytes(CORPUS)
        summary = Path(tmp) / "summary.md"
        env = {
            **os.environ,
            "LIMBO_URL": url or corpus.as_uri(),
            "LIMBO_SHA256": sha,
            "LIMBO_DEST": str(Path(tmp) / "limbo.json"),
            "GITHUB_STEP_SUMMARY": str(summary),
        }
        result = subprocess.run(
            ["bash", str(SCRIPT), "sh", "-c", command],
            capture_output=True,
            text=True,
            env=env,
            check=False,
        )
        return result, summary.read_text(encoding="utf-8") if summary.exists() else ""


PASS = 'test -f "$LIMBO_JSON" && echo "test result: ok. 1 passed; 0 failed"'


class LimboDifferentialTests(unittest.TestCase):
    def test_a_matching_corpus_runs_the_command_with_it(self) -> None:
        result, summary = run(PASS)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("1 passed", summary)

    def test_a_failing_command_fails_the_step(self) -> None:
        result, _ = run("echo looser; exit 101")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("looser", result.stdout)

    def test_a_zero_test_run_is_not_a_pass(self) -> None:
        result, _ = run('echo "test result: ok. 0 passed; 0 failed"')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("did not report 1 passed", result.stdout)

    def test_a_changed_corpus_skips_visibly_and_runs_nothing(self) -> None:
        result, summary = run("echo RAN; exit 1", sha="0" * 64)
        self.assertEqual(result.returncode, 0)
        self.assertNotIn("RAN", result.stdout)
        self.assertIn("::warning::x509-limbo differential SKIPPED", result.stdout)
        self.assertIn("SKIPPED", summary)

    def test_a_failed_download_skips_visibly_and_runs_nothing(self) -> None:
        result, summary = run("echo RAN; exit 1", url="file:///nonexistent/limbo.json")
        self.assertEqual(result.returncode, 0)
        self.assertNotIn("RAN", result.stdout)
        self.assertIn("could not download", result.stdout)
        self.assertIn("SKIPPED", summary)


if __name__ == "__main__":
    unittest.main()

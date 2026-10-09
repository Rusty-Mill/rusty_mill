"""Tests for live_internet_check.sh.

The script wraps a non-blocking CI step, so what it leaves behind (output, a
step summary, an exit status) is the whole of its value. These run it against
fake commands: no network, no cargo.
"""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("live_internet_check.sh")


def run(*command: str) -> tuple[subprocess.CompletedProcess[str], str]:
    with tempfile.TemporaryDirectory() as tmp:
        summary = Path(tmp) / "summary.md"
        env = {**os.environ, "GITHUB_STEP_SUMMARY": str(summary)}
        result = subprocess.run(
            ["bash", str(SCRIPT), *command],
            capture_output=True,
            text=True,
            env=env,
            cwd=tmp,
            check=False,
        )
        text = summary.read_text(encoding="utf-8") if summary.exists() else ""
    return result, text


def fake(output: str, code: int, stream: str = "1") -> tuple[str, ...]:
    return ("sh", "-c", f"printf '%s\\n' '{output}' >&{stream}; exit {code}")


class LiveInternetCheckTests(unittest.TestCase):
    def test_a_failing_command_still_prints_and_summarises_its_output(self) -> None:
        result, summary = run(*fake("accounts.google.com: handshake: boom", 101))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("accounts.google.com: handshake: boom", result.stdout)
        self.assertIn("accounts.google.com: handshake: boom", summary)
        self.assertIn("::warning::", result.stdout)
        self.assertIn("exit 101", result.stdout)

    def test_stderr_is_kept_because_compile_errors_live_there(self) -> None:
        result, summary = run(*fake("error: could not compile", 101, stream="2"))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("could not compile", result.stdout)
        self.assertIn("could not compile", summary)

    def test_one_passing_test_succeeds(self) -> None:
        result, summary = run(
            *fake("www.googleapis.com: ok\ntest result: ok. 1 passed; 0 failed", 0)
        )
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("test result: ok. 1 passed", summary)

    def test_a_zero_test_run_is_not_a_pass(self) -> None:
        # `cargo test` exits 0 when the cfg removes every test.
        result, summary = run(*fake("test result: ok. 0 passed; 0 failed", 0))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("did not report 1 passed", result.stdout)
        self.assertIn("0 passed", summary)

    def test_a_failure_that_also_prints_a_pass_line_still_fails(self) -> None:
        result, _ = run(*fake("test result: ok. 1 passed", 1))
        self.assertNotEqual(result.returncode, 0)

    def test_it_works_without_a_step_summary_file(self) -> None:
        env = {k: v for k, v in os.environ.items() if k != "GITHUB_STEP_SUMMARY"}
        result = subprocess.run(
            ["bash", str(SCRIPT), *fake("test result: ok. 1 passed", 0)],
            capture_output=True,
            text=True,
            env=env,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stdout)


if __name__ == "__main__":
    unittest.main()

"""Tests for the pinned-commit fetch retry in rusty_tls/bogo/run.sh.

`git` and `cargo` are fakes on PATH: no network, no build. The fake `cargo`
fails with a distinctive status, so reaching it proves the fetch step finished.
"""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).parents[2] / "crates/libs/net/rusty_tls/bogo/run.sh"
PAST_FETCH = 7  # the fake cargo's exit status

FAKE_GIT = """#!/bin/sh
case "$*" in
  *" fetch "*)
    echo fetch >> "$FAKE_LOG"
    n=$(grep -c fetch "$FAKE_LOG")
    if [ "$n" -le "$FAKE_FETCH_FAILURES" ]; then exit 128; fi
    exit 0 ;;
  *rev-parse*) echo not-the-pinned-commit ;;
esac
exit 0
"""


def run(failures: int) -> tuple[subprocess.CompletedProcess[str], int]:
    with tempfile.TemporaryDirectory() as tmp:
        bin_dir = Path(tmp) / "bin"
        bin_dir.mkdir()
        for name, body in (("git", FAKE_GIT), ("cargo", f"#!/bin/sh\nexit {PAST_FETCH}\n")):
            tool = bin_dir / name
            tool.write_text(body)
            tool.chmod(0o755)
        log = Path(tmp) / "log"
        log.touch()
        env = {
            **os.environ,
            "PATH": f"{bin_dir}:{os.environ['PATH']}",
            "OUT": str(Path(tmp) / "out"),
            "FAKE_LOG": str(log),
            "FAKE_FETCH_FAILURES": str(failures),
            "BOGO_FETCH_PAUSE": "0",
        }
        env.pop("BORINGSSL", None)
        result = subprocess.run(
            ["bash", str(SCRIPT)], capture_output=True, text=True, env=env, check=False
        )
        return result, log.read_text().count("fetch")


class BogoFetchRetryTests(unittest.TestCase):
    def test_a_first_try_success_fetches_once(self) -> None:
        result, fetches = run(0)
        self.assertEqual(result.returncode, PAST_FETCH, result.stderr)
        self.assertEqual(fetches, 1)

    def test_transient_failures_are_retried_until_it_succeeds(self) -> None:
        result, fetches = run(3)
        self.assertEqual(result.returncode, PAST_FETCH, result.stderr)
        self.assertEqual(fetches, 4)
        self.assertIn("attempt 3 of 5", result.stderr)

    def test_a_persistent_failure_still_fails_after_five_attempts(self) -> None:
        result, fetches = run(99)
        self.assertNotEqual(result.returncode, PAST_FETCH, "must not reach the build")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(fetches, 5)


if __name__ == "__main__":
    unittest.main()

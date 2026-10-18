"""The acceptance collector must never turn missing evidence into a pass."""

from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import orch_bbp_acceptance as acceptance


class AcceptanceTests(unittest.TestCase):
    def test_requires_one_executed_test_and_mode_evidence(self) -> None:
        good = "test result: ok. 1 passed; 0 failed; 0 ignored; 10 filtered out;\nBBP_ACCEPTANCE PASS kill\n"
        self.assertTrue(acceptance.assess("kill", 0, good).passed)
        for code, output in (
            (1, good), (0, good.replace("1 passed", "0 passed")),
            (0, good.replace("0 ignored", "1 ignored")),
            (0, good.replace("PASS kill", "PASS timeout")),
            (0, good + "Landlock filesystem confinement is NotEnforced\n"),
            (0, "BBP_ACCEPTANCE PASS kill\n"),
        ):
            with self.subTest(code=code, output=output):
                self.assertFalse(acceptance.assess("kill", code, output).passed)

    def test_timeout_and_spawn_failure_keep_their_logs(self) -> None:
        for error in (subprocess.TimeoutExpired("cargo", 180, output=b"partial log"),
                      OSError("cargo unavailable")):
            with self.subTest(error=error), tempfile.TemporaryDirectory() as directory:
                with patch.object(acceptance.subprocess, "run", side_effect=error):
                    result = acceptance.run_case(acceptance.CASES[0], Path(directory))
                self.assertFalse(result.passed)
                self.assertTrue((Path(directory) / "lifecycle.log").read_text())

    def test_failure_does_not_skip_later_modes_and_returns_failure(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            results = [acceptance.Result(c.mode, c.mode != "timeout", "fixture")
                       for c in acceptance.CASES]
            with patch("sys.argv", ["acceptance", "--output", directory]), \
                 patch.object(acceptance, "run_case", side_effect=results) as run:
                self.assertEqual(acceptance.main(), 1)
            self.assertEqual(run.call_count, len(acceptance.CASES))
            self.assertIn("| pause-adoption | PASS |", (Path(directory) / "summary.md").read_text())
            self.assertIn("| rust-lld | PASS |", (Path(directory) / "summary.md").read_text())
            self.assertIn('"mode": "kill"', (Path(directory) / "results.json").read_text())
            self.assertIn("| timeout | FAIL |", (Path(directory) / "summary.md").read_text())


if __name__ == "__main__":
    unittest.main()

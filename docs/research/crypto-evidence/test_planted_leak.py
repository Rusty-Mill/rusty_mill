"""Failure-injection tests for planted_leak.sh with a fake cargo.

Run: python3 docs/research/crypto-evidence/test_planted_leak.py (needs a POSIX sh).
The guards must fail on: a non-zero cargo exit, a mistyped filter that succeeds with
zero tests, more than one test, and a missing cargo.
"""
import os
import shutil
import stat
import subprocess
import tempfile
import unittest

SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "planted_leak.sh")


class PlantedLeak(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)

    def run_with(self, cargo_body, cargo=None):
        if cargo is None:
            cargo = os.path.join(self.dir, "cargo")
            with open(cargo, "w") as f:
                f.write("#!/bin/sh\n" + cargo_body)
            os.chmod(cargo, os.stat(cargo).st_mode | stat.S_IEXEC)
        return subprocess.run(["sh", SCRIPT], env={**os.environ, "CARGO": cargo},
                              capture_output=True, text=True)

    def test_one_passing_test_succeeds(self):
        r = self.run_with("echo 'test timing::tests::planted_early_exit_is_detected ... ok'\n"
                          "echo 'test result: ok. 1 passed; 0 failed; 1 filtered out'\n")
        self.assertEqual(r.returncode, 0, r.stdout)
        self.assertIn("detected", r.stdout)

    def test_a_failing_test_fails(self):
        r = self.run_with("echo 'test result: FAILED. 0 passed; 1 failed'\nexit 101\n")
        self.assertEqual(r.returncode, 1)
        self.assertIn("NOT detected", r.stdout)

    def test_a_filter_that_runs_zero_tests_fails_even_with_exit_zero(self):
        r = self.run_with("echo 'running 0 tests'\n"
                          "echo 'test result: ok. 0 passed; 0 failed; 0 ignored'\n")
        self.assertEqual(r.returncode, 1)
        self.assertIn("exactly one", r.stdout)

    def test_more_than_one_test_fails(self):
        r = self.run_with("echo 'test result: ok. 2 passed; 0 failed'\n")
        self.assertEqual(r.returncode, 1)

    def test_a_missing_cargo_fails(self):
        r = self.run_with("", cargo=os.path.join(self.dir, "no-such-cargo"))
        self.assertEqual(r.returncode, 1)


if __name__ == "__main__":
    unittest.main()

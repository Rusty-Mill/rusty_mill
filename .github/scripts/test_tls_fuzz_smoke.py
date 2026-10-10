"""Tests for tls_fuzz_smoke.sh against a fake cargo-fuzz: no nightly, no build."""

from __future__ import annotations

import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("tls_fuzz_smoke.sh")

FAKE = """#!/bin/sh
# usage: fake list | fake run <target> -- <flags>
case "$1" in
  list) [ -n "$FAKE_LIST_FAILS" ] && exit 2; printf '%s\\n' $FAKE_TARGETS ;;
  run)
    case " $FAKE_CRASH " in *" $2 "*) echo "SUMMARY: crash in $2"; exit 77 ;; esac
    case " $FAKE_SILENT " in *" $2 "*) echo "no runs here"; exit 0 ;; esac
    echo "args: $*"; echo "Done 1234 runs in 1 second(s)" ;;
esac
"""


def run(targets: str, min_targets: int = 2, **env: str) -> subprocess.CompletedProcess[str]:
    with tempfile.TemporaryDirectory() as tmp:
        fake = Path(tmp) / "fake"
        fake.write_text(FAKE, encoding="utf-8")
        fake.chmod(fake.stat().st_mode | stat.S_IEXEC)
        full = {
            **os.environ,
            "CARGO_FUZZ": str(fake),
            "FUZZ_ROOT": tmp,
            "FAKE_TARGETS": targets,
            **env,
        }
        return subprocess.run(
            ["bash", str(SCRIPT), "1", str(min_targets)],
            capture_output=True,
            text=True,
            env=full,
            check=False,
        )


class TlsFuzzSmokeTests(unittest.TestCase):
    def test_all_targets_running_passes(self) -> None:
        r = run("a b c")
        self.assertEqual(r.returncode, 0, r.stdout)
        self.assertIn("3 targets, 0 failed", r.stdout)

    def test_the_target_triple_is_passed_only_when_set(self) -> None:
        self.assertIn("run a --target x86_64-unknown-linux-gnu", run("a b", FUZZ_TARGET_TRIPLE="x86_64-unknown-linux-gnu").stdout)
        self.assertNotIn("--target", run("a b").stdout)

    def test_a_crash_fails_but_the_other_targets_still_run(self) -> None:
        r = run("a b c", FAKE_CRASH="b")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("fuzz target b failed (exit 77)", r.stdout)
        self.assertIn("fuzz target c: ok", r.stdout)

    def test_a_target_that_runs_nothing_fails(self) -> None:
        r = run("a b", FAKE_SILENT="a")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("fuzz target a ran nothing", r.stdout)

    def test_too_few_targets_fails(self) -> None:
        r = run("a", min_targets=2)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("expected at least 2", r.stdout)

    def test_no_targets_fails(self) -> None:
        r = run("", min_targets=1)
        self.assertNotEqual(r.returncode, 0)

    def test_a_failing_list_fails(self) -> None:
        r = run("a b", FAKE_LIST_FAILS="1")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("could not list", r.stdout)


if __name__ == "__main__":
    unittest.main()

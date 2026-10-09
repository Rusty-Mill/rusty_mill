"""Deterministic tests for timing_series.py with a fake `cargo` and fake test executables.

Run: python3 docs/research/crypto-evidence/test_timing_series.py
(from the repository root; needs only the standard library and a POSIX sh).
"""
import io
import json
import os
import stat
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import timing_series as ts  # noqa: E402

CALIBRATION = "null_calibration_identical_classes"


def write_exe(path, tests):
    """A fake test binary. `tests` maps name -> list of |t| values, replayed in order (cycling)."""
    body = f"""#!{sys.executable}
import os, sys
TESTS = {tests!r}
CALIBRATION = {CALIBRATION!r}
args = sys.argv[1:]
if args[:2] == ["--ignored", "--list"]:
    for name in TESTS:
        print(f"{{name}}: test")
    sys.exit(0)
name = args[2]
state = os.path.abspath(sys.argv[0]) + "." + name
n = int(open(state).read()) if os.path.exists(state) else 0
open(state, "w").write(str(n + 1))
v = TESTS[name][n % len(TESTS[name])]
print(f"{{name}}: |t| = {{v}}")
# Leak tests assert |t| < 4.5 and so fail at or above it; calibration tests only print.
sys.exit(101 if name != CALIBRATION and v >= 4.5 else 0)
"""
    with open(path, "w") as f:
        f.write(body)
    os.chmod(path, os.stat(path).st_mode | stat.S_IEXEC)


def write_cargo(path, artifacts, exit_code=0):
    """A fake cargo that prints compiler-artifact JSON for `artifacts` = {package: executable}."""
    msgs = []
    for pkg, exe in artifacts.items():
        msgs.append(json.dumps({
            "reason": "compiler-artifact",
            "package_id": f"path+file:///work/crates/foundation/{pkg}#0.1.0",
            "target": {"name": "timing", "kind": ["test"]},
            "profile": {"test": True},
            "executable": exe,
        }))
    # Noise that must be ignored: the library's own artifact and a non-JSON line.
    msgs.append(json.dumps({"reason": "compiler-artifact", "package_id": "x#1.0.0",
                            "target": {"name": "lib"}, "profile": {"test": False}, "executable": None}))
    msgs.append("not json")
    body = "\n".join(msgs).replace("'", "'\\''")
    with open(path, "w") as f:
        f.write(f"#!/bin/sh\nprintf '%s\\n' '{body}'\nexit {exit_code}\n")
    os.chmod(path, os.stat(path).st_mode | stat.S_IEXEC)


class Fixture:
    def __init__(self, case):
        self.dir = tempfile.mkdtemp()
        case.addCleanup(lambda: __import__("shutil").rmtree(self.dir, ignore_errors=True))

    def exe(self, name, tests, age=0):
        path = os.path.join(self.dir, name)
        write_exe(path, tests)
        os.utime(path, (1_000_000 + age, 1_000_000 + age))
        return path

    def cargo(self, artifacts, exit_code=0):
        path = os.path.join(self.dir, "cargo")
        write_cargo(path, artifacts, exit_code)
        return path


def run(cargo, specs, reps):
    out = io.StringIO()
    status = ts.run(specs, reps, cargo=cargo, out=out)
    return status, out.getvalue()


class SelectionTests(unittest.TestCase):
    def test_each_crate_uses_its_own_executable_whatever_the_mtimes(self):
        fx = Fixture(self)
        # Same-named calibration function in all three; the newest file belongs to the wrong crate.
        exes = {
            "rusty_sha2": fx.exe("timing-aaa", {CALIBRATION: [1.5]}, age=300),
            "rusty_aead": fx.exe("timing-bbb", {CALIBRATION: [2.5]}, age=100),
            "rusty_pk": fx.exe("timing-ccc", {CALIBRATION: [3.5]}, age=200),
        }
        specs = [(c, CALIBRATION, f"A/A {c}") for c in exes]
        status, out = run(fx.cargo(exes), specs, reps=3)
        self.assertEqual(status, 0, out)
        for crate, value in (("rusty_sha2", "1.5"), ("rusty_aead", "2.5"), ("rusty_pk", "3.5")):
            line = next(l for l in out.splitlines() if l.startswith(f"A/A {crate}:"))
            self.assertIn(f"min={value} ", line)
            self.assertIn(f"max={value} ", line)
        for exe in exes.values():
            self.assertIn(exe, out)  # the identity is recorded

    def test_an_executable_outside_any_target_directory_is_found(self):
        fx = Fixture(self)
        elsewhere = tempfile.mkdtemp()
        self.addCleanup(lambda: __import__("shutil").rmtree(elsewhere, ignore_errors=True))
        exe = os.path.join(elsewhere, "custom-target", "release", "deps", "timing-1")
        os.makedirs(os.path.dirname(exe))
        write_exe(exe, {CALIBRATION: [1.0]})
        status, out = run(fx.cargo({"rusty_pk": exe}), [("rusty_pk", CALIBRATION, "A/A pk")], 2)
        self.assertEqual(status, 0, out)

    def test_package_names_are_parsed_from_both_id_formats(self):
        self.assertEqual(ts.package_name("path+file:///w/crates/foundation/rusty_pk#0.1.0"), "rusty_pk")
        self.assertEqual(ts.package_name("registry+https://x/index#rusty_pk@0.1.0"), "rusty_pk")


class FailureTests(unittest.TestCase):
    def test_a_missing_test_fails_the_record(self):
        fx = Fixture(self)
        exe = fx.exe("timing-1", {CALIBRATION: [1.0]})
        status, out = run(fx.cargo({"rusty_pk": exe}), [("rusty_pk", "no_such_test", "leak pk")], 2)
        self.assertEqual(status, 1)
        self.assertIn("!! FAILED: leak pk: test no_such_test not found", out)

    def test_a_failed_build_fails_even_with_an_old_executable_present(self):
        fx = Fixture(self)
        old = fx.exe("timing-old", {CALIBRATION: [1.0]})
        status, out = run(fx.cargo({"rusty_pk": old}, exit_code=101), [("rusty_pk", CALIBRATION, "A/A pk")], 2)
        self.assertEqual(status, 1)
        self.assertIn("building the timing test failed", out)
        self.assertNotIn("min=", out)  # the stale binary was not run

    def test_no_artifact_for_the_package_fails(self):
        fx = Fixture(self)
        other = fx.exe("timing-1", {CALIBRATION: [1.0]})
        status, out = run(fx.cargo({"rusty_sha2": other}), [("rusty_pk", CALIBRATION, "A/A pk")], 2)
        self.assertEqual(status, 1)
        self.assertIn("expected exactly one timing test executable", out)

    def test_zero_repetitions_fail(self):
        fx = Fixture(self)
        exe = fx.exe("timing-1", {CALIBRATION: [1.0]})
        status, out = run(fx.cargo({"rusty_pk": exe}), [("rusty_pk", CALIBRATION, "A/A pk")], 0)
        self.assertEqual(status, 1)
        self.assertIn("REPS must be a positive number", out)

    def test_runs_that_print_no_statistic_fail(self):
        fx = Fixture(self)
        exe = os.path.join(fx.dir, "timing-1")
        with open(exe, "w") as f:  # lists the test, prints nothing when run
            f.write('#!/bin/sh\n[ "$2" = --list ] && echo "t: test"\nexit 0\n')
        os.chmod(exe, 0o755)
        status, out = run(fx.cargo({"rusty_pk": exe}), [("rusty_pk", "t", "A/A pk")], 3)
        self.assertEqual(status, 1)
        self.assertIn("0 of 3 runs produced a |t|", out)

    def test_a_leak_series_without_a_baseline_fails(self):
        fx = Fixture(self)
        exe = fx.exe("timing-1", {"leak": [1.0]})
        status, out = run(fx.cargo({"rusty_pk": exe}), [("rusty_pk", "leak", "leak pk")], 2)
        self.assertEqual(status, 1)
        self.assertIn("no A/A baseline", out)


class BaselineTests(unittest.TestCase):
    """Calibration tests exit 0 whatever they print, so alarms must come from the numbers."""

    # 40 runs: 10 of them above the threshold.
    NOISY = [20.0] * 10 + [1.0] * 30

    def specs(self):
        return [("rusty_pk", CALIBRATION, "A/A pk"), ("rusty_pk", "leak", "leak pk")]

    def go(self, leak_values):
        fx = Fixture(self)
        exe = fx.exe("timing-1", {CALIBRATION: self.NOISY, "leak": leak_values})
        return run(fx.cargo({"rusty_pk": exe}), self.specs(), reps=40)

    def test_above_threshold_calibration_output_counts_as_baseline_alarms(self):
        status, out = self.go([1.0] * 40)
        self.assertEqual(status, 0, out)
        self.assertIn("at_or_above_4.5=10", next(l for l in out.splitlines() if l.startswith("A/A pk:")))

    def test_the_allowance_is_twice_the_numeric_baseline_plus_one(self):
        ok, out = self.go([9.0] * 21 + [1.0] * 19)      # 21 alarms == 2*10+1
        self.assertEqual(ok, 0, out)
        bad, out = self.go([9.0] * 22 + [1.0] * 18)     # 22 alarms
        self.assertEqual(bad, 1)
        self.assertIn("22 alarms of 40 exceeds the baseline allowance (21; baseline 10)", out)

    def test_a_quiet_baseline_leaves_a_tight_allowance(self):
        fx = Fixture(self)
        exe = fx.exe("timing-1", {CALIBRATION: [1.0], "leak": [9.0] * 2 + [1.0] * 38})
        status, out = run(fx.cargo({"rusty_pk": exe}), self.specs(), reps=40)
        self.assertEqual(status, 1)
        self.assertIn("exceeds the baseline allowance (1; baseline 0)", out)


if __name__ == "__main__":
    unittest.main()

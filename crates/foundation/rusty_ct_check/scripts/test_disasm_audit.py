"""Tests for disasm_audit.verdict: the jump count is pinned exactly (Codex round 3 on #540).

Run: python3 crates/foundation/rusty_ct_check/scripts/test_disasm_audit.py
"""
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from disasm_audit import verdict  # noqa: E402


class Verdict(unittest.TestCase):
    def test_exact_count_passes(self):
        self.assertTrue(verdict(10, 0, 10))
        self.assertTrue(verdict(0, 0, 0))

    def test_a_higher_count_fails(self):
        self.assertFalse(verdict(11, 0, 10))

    def test_a_lower_count_fails_too(self):
        # Drift detection: fewer jumps is as much a change to review as more. (A same-count
        # substitution is NOT caught by any count rule; see the module docstring.)
        self.assertFalse(verdict(9, 0, 10))
        self.assertFalse(verdict(0, 0, 1))

    def test_any_division_fails(self):
        self.assertFalse(verdict(10, 1, 10))


SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "disasm_audit.py")

# Two functions: `leaf` has 1 conditional jump (jne) and an unconditional jmp (ignored), `div_fn` has a div.
LISTING = """0000000000001000 <rusty::leaf>:
    1000:\tcmp    %rax,%rbx
    1003:\tjne    1010 <rusty::leaf+0x10>
    1005:\tjmp    1020 <rusty::leaf+0x20>
    1007:\tret

0000000000002000 <rusty::div_fn>:
    2000:\tdiv    %rcx
    2003:\tret
"""


class EndToEnd(unittest.TestCase):
    """The script with a fake `objdump` on PATH: missing symbols and tool failure must fail."""

    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)
        self.binary = os.path.join(self.dir, "bin")
        open(self.binary, "w").close()

    def run_audit(self, limits, objdump_body=None):
        limits_path = os.path.join(self.dir, "limits.txt")
        with open(limits_path, "w") as f:
            f.write(limits)
        fake = os.path.join(self.dir, "objdump")
        body = objdump_body if objdump_body is not None else "cat <<'EOF'\n" + LISTING + "EOF\n"
        with open(fake, "w") as f:
            f.write("#!/bin/sh\n" + body)
        os.chmod(fake, os.stat(fake).st_mode | stat.S_IEXEC)
        env = {**os.environ, "PATH": self.dir + os.pathsep + os.environ["PATH"]}
        return subprocess.run([sys.executable, "-I", SCRIPT, self.binary, limits_path],
                              env=env, capture_output=True, text=True)

    def test_the_exact_reviewed_count_passes(self):
        r = self.run_audit("rusty::leaf 1\n")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_a_higher_or_lower_count_fails(self):
        self.assertEqual(self.run_audit("rusty::leaf 0\n").returncode, 1)
        self.assertEqual(self.run_audit("rusty::leaf 2\n").returncode, 1)

    def test_a_symbol_matching_nothing_fails(self):
        r = self.run_audit("rusty::leaf 1\nrusty::renamed_away 0\n")
        self.assertEqual(r.returncode, 1)
        self.assertIn("no function matches", r.stdout)

    def test_a_division_fails_whatever_the_jump_count(self):
        self.assertEqual(self.run_audit("rusty::div_fn 0\n").returncode, 1)

    def test_a_failing_objdump_fails_instead_of_passing_vacuously(self):
        r = self.run_audit("rusty::leaf 1\n", objdump_body="echo boom >&2\nexit 1\n")
        self.assertNotEqual(r.returncode, 0)


if __name__ == "__main__":
    unittest.main()

"""Tests for disasm_audit.verdict: the jump count is pinned exactly (Codex round 3 on #540).

Run: python3 crates/foundation/rusty_ct_check/scripts/test_disasm_audit.py
"""
import os
import sys
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
        # A removed bounds check or loop could be masking an added secret-dependent jump.
        self.assertFalse(verdict(9, 0, 10))
        self.assertFalse(verdict(0, 0, 1))

    def test_any_division_fails(self):
        self.assertFalse(verdict(10, 1, 10))


if __name__ == "__main__":
    unittest.main()

"""Tests for verdict_sites.py on synthetic disassembly (no objdump or valgrind needed).

Run: python3 -I crates/foundation/rusty_pk/scripts/test_verdict_sites.py
"""
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import verdict_sites as vs  # noqa: E402

BASE = vs.BASE


def disasm(*rows):
    """rows: (file_addr, function, mnemonic, operands)."""
    insns = [(a, op, args) for a, _, op, args in rows]
    func_of = {a: f for a, f, _, _ in rows}
    index = {a: n for n, (a, _, _) in enumerate(insns)}
    return insns, func_of, index


F = "<rusty_pk::ecdh::PrivateKey>::agree"
ENSURE = "1b2d0 <rusty_pk::ecdh::ensure>"


class SiteTests(unittest.TestCase):
    def check(self, rows, at, name=F):
        insns, func_of, index = disasm(*rows)
        return vs.check_site(BASE + at, name, insns, func_of, index)

    def test_a_jump_right_after_the_ensure_call_is_the_verdict(self):
        rows = [(0x10, F, "call", ENSURE), (0x15, F, "test", "%al,%al"), (0x17, F, "jne", "188e8")]
        self.assertIsNone(self.check(rows, 0x17))

    def test_one_unrelated_instruction_between_is_fine(self):
        rows = [(0x10, F, "call", ENSURE), (0x15, F, "mov", "$0x1,%r13d"),
                (0x1b, F, "test", "%al,%al"), (0x1d, F, "jne", "188e8")]
        self.assertIsNone(self.check(rows, 0x1d))

    def test_a_jump_after_another_call_is_not(self):
        rows = [(0x10, F, "call", "1a360 <rusty_pk::ecdh::mul>"), (0x15, F, "test", "%al,%al"),
                (0x17, F, "jne", "188e8")]
        self.assertIn("not ensure", self.check(rows, 0x17))

    def test_a_jump_inside_a_loop_is_not(self):
        rows = [(0x10, F, "cmp", "$0x4,%r15"), (0x14, F, "je", "1813e"), (0x1a, F, "mov", "%rax,%rbx"),
                (0x1d, F, "jne", "18042")]
        self.assertIn("no call to ecdh::ensure", self.check(rows, 0x1d))

    def test_an_intervening_jump_hides_an_earlier_ensure_call(self):
        rows = [(0x10, F, "call", ENSURE), (0x15, F, "je", "18000"), (0x17, F, "test", "%al,%al"),
                (0x19, F, "jne", "188e8")]
        self.assertIn("no call to ecdh::ensure", self.check(rows, 0x19))

    def test_a_non_jump_or_a_missing_address_or_a_wrong_name_fails(self):
        rows = [(0x10, F, "call", ENSURE), (0x15, F, "mov", "%rax,%rbx")]
        self.assertIn("not a conditional jump", self.check(rows, 0x15))
        self.assertIn("no instruction", self.check(rows, 0x99))
        rows = [(0x10, F, "call", ENSURE), (0x15, F, "jne", "x")]
        self.assertIn("report says", self.check(rows, 0x15, name="taint::main"))


class ReportTests(unittest.TestCase):
    def test_only_the_top_frame_of_each_diagnostic_is_taken(self):
        text = (
            "==1== Conditional jump or move depends on uninitialised value(s)\n"
            "==1==    at 0x120491: <rusty_pk::ecdh::PrivateKey>::public_key (in /x/taint)\n"
            "==1==    by 0x11D726: taint::main (in /x/taint)\n"
            "==1== \n"
            "==1== Use of uninitialised value of size 8\n"
            "==1==    at 0x120C4E: <rusty_pk::ecdh::PrivateKey>::agree (in /x/taint)\n"
        )
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
            f.write(text)
        self.addCleanup(os.unlink, f.name)
        self.assertEqual([hex(a) for a, _ in vs.reports(f.name)], ["0x120491", "0x120c4e"])


if __name__ == "__main__":
    unittest.main()

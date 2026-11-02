"""Tests for verdict_sites.py on synthetic disassembly (no objdump or valgrind needed).

Run: python3 -I crates/foundation/rusty_pk/scripts/test_verdict_sites.py
"""
import contextlib
import io
import os
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import verdict_sites as vs  # noqa: E402

BASE = 0x108000  # valgrind 3.22's PIE placement; nothing in the checker assumes it any more


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
        return vs.check_site(BASE + at, name, insns, func_of, index, BASE)

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
        self.assertIn("report says", self.check(rows, 0x15, name="rusty_pk_taint::main"))


class ReportTests(unittest.TestCase):
    def test_only_the_top_frame_of_each_diagnostic_is_taken(self):
        text = (
            "==1== Conditional jump or move depends on uninitialised value(s)\n"
            "==1==    at 0x120491: <rusty_pk::ecdh::PrivateKey>::public_key (in /x/rusty_pk_taint)\n"
            "==1==    by 0x11D726: rusty_pk_taint::main (in /x/rusty_pk_taint)\n"
            "==1== \n"
            "==1== Use of uninitialised value of size 8\n"
            "==1==    at 0x120C4E: <rusty_pk::ecdh::PrivateKey>::agree (in /x/rusty_pk_taint)\n"
        )
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
            f.write(text)
        self.addCleanup(os.unlink, f.name)
        found = vs.reports(f.name)
        self.assertEqual([hex(a) for a, _, _ in found], ["0x120491", "0x120c4e"])
        self.assertEqual(found[0][2], ["<rusty_pk::ecdh::PrivateKey>::public_key", "rusty_pk_taint::main"])
        self.assertEqual(found[1][2], ["<rusty_pk::ecdh::PrivateKey>::agree"])


class OutputTests(unittest.TestCase):
    STACK = ["memmove", "rusty_pk::ecdsa::der_signature::{closure#0}", "rusty_pk::ecdsa::output",
             "<rusty_pk::ecdsa::SigningKey>::sign_seeded"]

    def test_a_stack_through_an_output_function_is_accepted(self):
        self.assertEqual(vs.through_output(self.STACK, ["ecdsa::output"]), "ecdsa::output")

    def test_without_the_output_frame_or_the_option_it_is_not(self):
        self.assertIsNone(vs.through_output(self.STACK[:2] + self.STACK[3:], ["ecdsa::output"]))
        self.assertIsNone(vs.through_output(self.STACK, []))


def write(test, text):
    with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
        f.write(text)
    test.addCleanup(os.unlink, f.name)
    return f.name


MAIN = "rusty_pk_taint::main"
STARTS = {MAIN: [0x1d700], F: [0x20400]}


class MappingTests(unittest.TestCase):
    def test_the_base_is_the_anchor_minus_its_symbol(self):
        self.assertEqual(vs.load_base([(MAIN, 0x125700)], STARTS), 0x108000)
        self.assertEqual(vs.load_base([(MAIN, 0x401d700)], STARTS), 0x4000000)
        self.assertEqual(vs.load_base([(MAIN, 0x1d700)], STARTS), 0)  # non-PIE

    def test_a_missing_or_repeated_anchor_is_rejected(self):
        with self.assertRaisesRegex(vs.MappingError, "found 0"):
            vs.load_base([], STARTS)
        with self.assertRaisesRegex(vs.MappingError, "found 2"):
            vs.load_base([(MAIN, 0x125700), (MAIN, 0x125700)], STARTS)
        with self.assertRaisesRegex(vs.MappingError, "found 2"):
            vs.load_base([(MAIN, 0x125700), (MAIN, 0x401d700)], STARTS)

    def test_an_absent_or_ambiguous_symbol_is_rejected(self):
        with self.assertRaisesRegex(vs.MappingError, "appears 0 times"):
            vs.load_base([("rusty_pk_taint::other", 0x125700)], STARTS)
        with self.assertRaisesRegex(vs.MappingError, "appears 2 times"):
            vs.load_base([(MAIN, 0x125700)], {MAIN: [0x1d700, 0x1e000]})

    def test_a_negative_or_unaligned_base_is_rejected(self):
        with self.assertRaisesRegex(vs.MappingError, "not a non-negative multiple"):
            vs.load_base([(MAIN, 0x1d000)], STARTS)
        with self.assertRaisesRegex(vs.MappingError, "not a non-negative multiple"):
            vs.load_base([(MAIN, 0x125704)], STARTS)

    def test_anchor_lines_are_read_exactly(self):
        path = write(self, "verdict_sites: load anchor rusty_pk_taint::main at 0x401d700\n"
                           "==1== verdict_sites: load anchor rusty_pk_taint::main at 0x1\n"
                           "verdict_sites: load anchor rusty_pk_taint::main at 0x2 trailing\n")
        self.assertEqual(vs.anchors(path), [(MAIN, 0x401d700)])

    def test_disassembly_heads_record_every_start(self):
        out = ("000000000001d700 <rusty_pk_taint::main>:\n"
               "   1d700:\tpush   %rbp\n"
               "0000000000020400 <rusty_pk::ecdh::ensure>:\n"
               "   20400:\tret\n")
        insns, func_of, starts = vs.parse_disassembly(out)
        self.assertEqual(starts, {MAIN: [0x1d700], "rusty_pk::ecdh::ensure": [0x20400]})
        self.assertEqual(func_of[0x20400], "rusty_pk::ecdh::ensure")
        self.assertEqual([a for a, _, _ in insns], [0x1d700, 0x20400])


DISASM = ("000000000001d700 <rusty_pk_taint::main>:\n"
          "   1d700:\tpush   %rbp\n"
          "0000000000020400 <<rusty_pk::ecdh::PrivateKey>::agree>:\n"
          "   20400:\tcall   21000 <rusty_pk::ecdh::ensure>\n"
          "   20405:\ttest   %al,%al\n"
          "   20407:\tjne    20500\n")


class EndToEndTests(unittest.TestCase):
    """main() on a report from each load base, with objdump replaced by DISASM."""

    def run_main(self, report):
        path = write(self, report)
        out = io.StringIO()
        with mock.patch.object(vs, "disassemble", lambda _: vs.parse_disassembly(DISASM)), \
                contextlib.redirect_stdout(out), self.assertRaises(SystemExit) as exit:
            vs.main(["verdict_sites.py", "bin", path, "1"])
        return exit.exception.code, out.getvalue()

    @staticmethod
    def report(base, anchor=True):
        head = f"verdict_sites: load anchor rusty_pk_taint::main at 0x{base + 0x1d700:x}\n" if anchor else ""
        return head + (
            "==1== Conditional jump or move depends on uninitialised value(s)\n"
            f"==1==    at 0x{base + 0x20407:X}: <rusty_pk::ecdh::PrivateKey>::agree (in /x/rusty_pk_taint)\n")

    def test_the_verdict_passes_at_the_old_and_a_moved_base(self):
        for base in (0x108000, 0x4000000):
            code, out = self.run_main(self.report(base))
            self.assertEqual(code, 0, out)
            self.assertIn(f"load base 0x{base:x}", out)

    def test_a_report_from_another_base_than_its_anchor_fails(self):
        code, out = self.run_main(self.report(0x4000000).replace("0x401d700", "0x125700"))
        self.assertEqual(code, 1)
        self.assertIn("no instruction", out)

    def test_no_anchor_fails_before_any_report_is_classified(self):
        code, out = self.run_main(self.report(0x108000, anchor=False))
        self.assertEqual(code, 1)
        self.assertIn("FAIL load mapping", out)
        self.assertNotIn("report at", out)


if __name__ == "__main__":
    unittest.main()

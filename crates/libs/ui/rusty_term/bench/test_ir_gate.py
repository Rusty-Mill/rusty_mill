"""Unit tests for ir_gate.py (run: python3 -m unittest discover -s bench -p 'test_*.py')."""

import unittest

from ir_gate import check, parse_ceilings, parse_refs


class ParseCeilings(unittest.TestCase):
    def test_comments_blanks_and_values(self):
        text = "# header\n\nascii_throughput 120.5  # note\nscroll_heavy 900\n"
        self.assertEqual(parse_ceilings(text), [("ascii_throughput", 120.5), ("scroll_heavy", 900.0)])

    def test_rejects_malformed_lines(self):
        for bad in ("ascii_throughput", "a 1 2", "a fast", "a 0", "a -3"):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                parse_ceilings(bad)

    def test_rejects_an_empty_file(self):
        with self.assertRaises(ValueError):
            parse_ceilings("# only comments\n\n")


class ParseRefs(unittest.TestCase):
    def test_reads_the_cachegrind_total(self):
        stderr = "==1== Cachegrind, a high-precision tracing profiler\n==1== I refs:        73,714,730\n"
        self.assertEqual(parse_refs(stderr), 73_714_730)

    def test_missing_total_is_an_error(self):
        with self.assertRaises(ValueError):
            parse_refs("==1== something else\n")


class Check(unittest.TestCase):
    def test_flags_over_ceiling_and_unmeasured(self):
        ceilings = [("a", 100.0), ("b", 50.0), ("c", 10.0)]
        table, failures = check({"a": 90.0, "b": 51.0}, ceilings)
        self.assertEqual(len(failures), 2)
        self.assertTrue(failures[0].startswith("b:"))
        self.assertIn("not measured", failures[1])
        self.assertIn("| a | 90.0 | 100.0 | ok |", table)

    def test_at_the_ceiling_passes(self):
        self.assertEqual(check({"a": 100.0}, [("a", 100.0)])[1], [])


if __name__ == "__main__":
    unittest.main()

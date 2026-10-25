"""Unit tests for floors.py (run: python3 -m unittest discover -s bench -p 'test_*.py')."""

import unittest

from floors import check, parse_mbps

SAMPLE = """--- rusty_term parser+grid throughput (120x40 grid, 4096B chunks, x10 each) ---
ascii_throughput          4000045 bytes  x10       330.1 ms total      121.2 MB/s
                       chunk latency (n=9770  ) p50=  30000 p95=  40000 p99=  50000 p99.9=  60000 (ns/4096B chunk)
"""


class ParseMbps(unittest.TestCase):
    def test_reads_the_summary_line_not_the_latency_line(self):
        self.assertEqual(parse_mbps(SAMPLE), {"ascii_throughput": 121.2})

    def test_no_summary_line(self):
        self.assertEqual(parse_mbps("skip x.vt: empty file\n"), {})


class Check(unittest.TestCase):
    def test_judges_the_slowest_run(self):
        table, failures = check({"a": [70.0, 57.9, 90.0]}, {"a": 58.0})
        self.assertEqual(failures, ["a: 57.9 MB/s is below the 58.0 floor"])
        self.assertIn("| a | 57.9 | 70.0, 57.9, 90.0 | 58.0 | **below floor** |", table)

    def test_at_the_floor_passes(self):
        self.assertEqual(check({"a": [58.0]}, {"a": 58.0})[1], [])

    def test_unmeasured_fails(self):
        self.assertIn("not measured", check({"a": []}, {"a": 1.0})[1][0])


if __name__ == "__main__":
    unittest.main()

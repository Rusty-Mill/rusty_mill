"""Tests for fuzz_seconds.py, the long fuzz run's duration guard."""

from __future__ import annotations

import subprocess
import sys
import unittest
from pathlib import Path

from fuzz_seconds import MAX_SECONDS, parse

SCRIPT = Path(__file__).with_name("fuzz_seconds.py")


class FuzzSecondsTests(unittest.TestCase):
    def test_both_boundaries_are_accepted(self) -> None:
        self.assertEqual(parse("1"), 1)
        self.assertEqual(parse(str(MAX_SECONDS)), MAX_SECONDS)

    def test_zero_is_refused_because_libfuzzer_reads_it_as_unlimited(self) -> None:
        for value in ("0", "00", "0010800"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                parse(value)

    def test_one_past_the_limit_is_refused(self) -> None:
        with self.assertRaises(ValueError):
            parse(str(MAX_SECONDS + 1))

    def test_oversized_decimal_strings_are_refused_without_parsing(self) -> None:
        for value in ("99999", "9" * 40):
            with self.subTest(value=value[:8]), self.assertRaises(ValueError):
                parse(value)

    def test_malformed_input_is_refused(self) -> None:
        for value in ("", " 60", "60 ", "-60", "+60", "1e4", "3h", "１２"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                parse(value)

    def test_the_command_exits_non_zero_on_a_refusal(self) -> None:
        ok = subprocess.run([sys.executable, str(SCRIPT), "10800"], capture_output=True, text=True, check=False)
        self.assertEqual((ok.returncode, ok.stdout.strip()), (0, "10800"))
        bad = subprocess.run([sys.executable, str(SCRIPT), "0"], capture_output=True, text=True, check=False)
        self.assertEqual(bad.returncode, 1)
        self.assertIn("::error::", bad.stdout)


if __name__ == "__main__":
    unittest.main()

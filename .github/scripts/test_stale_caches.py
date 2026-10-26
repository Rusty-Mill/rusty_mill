"""Tests for stale_caches.py."""

import json
import subprocess
import sys
import unittest
from pathlib import Path

from stale_caches import Cache, parse, superseded

SCRIPT = Path(__file__).with_name("stale_caches.py")


class SupersededTests(unittest.TestCase):
    def test_keeps_only_the_newest_entry_per_family(self) -> None:
        caches = [
            Cache(1, "v0-rust-test-ubuntu-latest-Linux-x64-a49c5372-d0b8688b", "2026-10-25T08:53:00Z"),
            Cache(2, "v0-rust-test-ubuntu-latest-Linux-x64-a49c5372-05a855ff", "2026-10-26T07:40:00Z"),
            Cache(3, "v0-rust-clippy-ubuntu-latest-Linux-x64-a49c5372-05a855ff", "2026-10-26T07:30:00Z"),
        ]
        self.assertEqual(superseded(caches), [1])

    def test_a_new_environment_hash_is_a_separate_family(self) -> None:
        # A runner image update changes the environment hash; both stay until LRU decides.
        caches = [
            Cache(1, "v0-rust-test-windows-latest-Windows_NT-x64-099b9200-d0b8688b", "2026-10-25T08:00:00Z"),
            Cache(2, "v0-rust-test-windows-latest-Windows_NT-x64-11111111-05a855ff", "2026-10-26T08:00:00Z"),
        ]
        self.assertEqual(superseded(caches), [])

    def test_equal_timestamps_keep_the_higher_id(self) -> None:
        caches = [Cache(7, "v0-rust-plan-Linux-x64-a-1", "t"), Cache(9, "v0-rust-plan-Linux-x64-a-2", "t")]
        self.assertEqual(superseded(caches), [7])

    def test_empty_list_deletes_nothing(self) -> None:
        self.assertEqual(superseded([]), [])

    def test_cli_reads_gh_json_and_prints_ids(self) -> None:
        rows = [
            {"id": 11, "key": "v0-rust-plan-Linux-x64-a49c5372-d0b8688b", "createdAt": "2026-10-25T08:00:00Z"},
            {"id": 12, "key": "v0-rust-plan-Linux-x64-a49c5372-05a855ff", "createdAt": "2026-10-26T08:00:00Z"},
        ]
        out = subprocess.run(
            [sys.executable, str(SCRIPT)], input=json.dumps(rows), capture_output=True, text=True, check=True
        ).stdout
        self.assertEqual(out, "11\n")

    def test_malformed_input_fails_loudly(self) -> None:
        with self.assertRaises(KeyError):
            parse('[{"id": 1}]')


if __name__ == "__main__":
    unittest.main()

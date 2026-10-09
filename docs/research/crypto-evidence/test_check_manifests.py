"""Tests for check_manifests.py. Run: python3 docs/research/crypto-evidence/test_check_manifests.py"""
import hashlib
import os
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from check_manifests import check  # noqa: E402


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class Manifests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)
        self.write("a.json", b'{"a":1}')
        self.write("b.txt", b"b")
        self.manifest({"a.json": b'{"a":1}', "b.txt": b"b"})

    def write(self, name, data):
        with open(os.path.join(self.dir, name), "wb") as f:
            f.write(data)

    def manifest(self, pins):
        lines = ["Source: prose line that is not a hash line"]
        lines += [f"{sha(d)}  {n}" for n, d in pins.items()]
        self.write("MANIFEST.txt", ("\n".join(lines) + "\n").encode())

    def test_matching_directory_passes(self):
        self.assertEqual(check(self.dir), (2, []))

    def test_an_edited_file_fails(self):
        self.write("a.json", b'{"a":2}')
        n, problems = check(self.dir)
        self.assertEqual(n, 2)
        self.assertEqual(len(problems), 1)
        self.assertIn("a.json hash", problems[0])

    def test_a_missing_file_fails(self):
        os.remove(os.path.join(self.dir, "b.txt"))
        self.assertTrue(any("b.txt is listed in MANIFEST.txt but missing" in p for p in check(self.dir)[1]))

    def test_an_unlisted_file_fails(self):
        self.write("c.json", b"{}")
        self.assertTrue(any("c.json is not listed" in p for p in check(self.dir)[1]))

    def test_whitespace_only_drift_in_a_json_vector_fails(self):
        # Same JSON value, one extra trailing newline: still a different file.
        self.write("a.json", b'{"a":1}\n')
        self.assertTrue(any("a.json hash" in p for p in check(self.dir)[1]))

    def test_a_changed_generated_text_corpus_fails(self):
        self.write("b.txt", b"b\n")
        self.assertTrue(any("b.txt hash" in p for p in check(self.dir)[1]))

    def test_a_file_listed_twice_fails(self):
        with open(os.path.join(self.dir, "MANIFEST.txt"), "a") as f:
            f.write(f"{sha(b'b')}  b.txt\n")
        self.assertTrue(any("b.txt is listed twice" in p for p in check(self.dir)[1]))

    def test_a_malformed_pin_line_fails_instead_of_reading_as_prose(self):
        with open(os.path.join(self.dir, "MANIFEST.txt"), "a") as f:
            f.write(f"{sha(b'x')} c.json\n")  # one space, not two
        n, problems = check(self.dir)
        self.assertTrue(any("malformed" in p for p in problems))

    def test_a_missing_or_empty_manifest_fails(self):
        self.write("MANIFEST.txt", b"only prose\n")
        self.assertTrue(any("pins no files" in p for p in check(self.dir)[1]))
        os.remove(os.path.join(self.dir, "MANIFEST.txt"))
        self.assertTrue(any("no MANIFEST.txt" in p for p in check(self.dir)[1]))


if __name__ == "__main__":
    unittest.main()

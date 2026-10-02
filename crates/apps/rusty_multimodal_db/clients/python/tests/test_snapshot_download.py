"""CSN-FR-005, ADR-0136 (protocol 36): the chunked snapshot download against a
scripted ``_roundtrip`` — no server. A manifest name that is not one plain file
name on every platform is refused before any file is created or truncated;
ordinary snapshot names still download and are checked against the manifest."""

import hashlib
import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))

from rusty_multimodal_db import protocol as p  # noqa: E402
from rusty_multimodal_db.client import Client, ProtocolError, is_plain_file_name  # noqa: E402

GOOD = b"abc"


def digest(data: bytes) -> bytes:
    return hashlib.sha256(data).digest()


class Scripted(Client):
    """A ``Client`` whose wire is a script: answers a manifest of ``files``,
    every chunk as ``chunk``, and records each request it was asked to send."""

    def __init__(self, files, chunk=GOOD):
        self.files, self.chunk, self.asked = files, chunk, []

    def _roundtrip(self, request):
        self.asked.append(request)
        if isinstance(request, p.BeginSnapshot):
            return p.SnapshotManifest(1, None, tuple(self.files))
        if isinstance(request, p.FetchChunk):
            return p.Chunk(self.chunk)
        return p.Ok()


class PlainFileName(unittest.TestCase):
    def test_ordinary_snapshot_names_pass(self):
        for name in [
            "memories.mmap",
            "memories.mmap.records",
            "memories.mmap.works-with_2.edges",
            "entities.mmap.mentioned_with.edges",
            "a b.c",
            "CONSOLE",
            "COM10",
            "Ünïcode.mmap",
            "a" * 255,
        ]:
            self.assertTrue(is_plain_file_name(name), repr(name))

    def test_names_that_could_escape_or_mean_something_else_are_refused(self):
        for name in [
            "", ".", "..", "...", "C:escape", "C:", "c:x", "C:\\escape",
            "\\\\server\\share\\x", "\\\\?\\C:\\x", "/abs", "a/b", "a\\b", "../x",
            "..\\x", "x/", "x:stream", "a*b", "a?b", 'a"b', "a<b", "a>b", "a|b",
            "nul\0", "tab\t", "trail.", "trail ", "CON", "con", "Nul.txt", "AUX .log",
            "COM1", "com9.x", "LPT0", "COM\u00b9", "CONIN$", "CONOUT$.x", "a" * 256,
        ]:
            self.assertFalse(is_plain_file_name(name), repr(name))


class ChunkedDownload(unittest.TestCase):
    def test_a_hostile_name_is_refused_before_anything_is_written(self):
        for bad in ["C:escape", "c:\\windows\\x", "\\\\server\\share\\x", "/etc/passwd",
                    "..\\x", "sub/x", "..", "x:stream", "NUL", "con.txt"]:
            with tempfile.TemporaryDirectory() as target:
                client = Scripted([("good", 3, digest(GOOD)), (bad, 3, digest(GOOD))])
                with self.assertRaises(ProtocolError, msg=bad):
                    client.fetch_snapshot_chunked(target)
                self.assertEqual(os.listdir(target), [], bad)
                self.assertFalse(any(isinstance(r, p.FetchChunk) for r in client.asked), bad)
                self.assertTrue(
                    any(isinstance(r, p.EndSnapshot) and r.snapshot == 1 for r in client.asked),
                    f"{bad}: the staged copy is still released",
                )

    def test_ordinary_names_download_into_the_directory(self):
        names = ["memories.mmap", "memories.mmap.records", "memories.mmap.knows.edges", "a b.c"]
        with tempfile.TemporaryDirectory() as target:
            client = Scripted([(n, 3, digest(GOOD)) for n in names])
            written, position = client.fetch_snapshot_chunked(target)
            self.assertEqual((written, position), (12, None))
            for name in names:
                with open(os.path.join(target, name), "rb") as f:
                    self.assertEqual(f.read(), GOOD, name)
            self.assertIsInstance(client.asked[-1], p.EndSnapshot)

    def test_a_chunk_that_does_not_match_the_digest_is_refused_and_released(self):
        with tempfile.TemporaryDirectory() as target:
            client = Scripted([("t", 3, digest(GOOD))], chunk=b"abd")
            with self.assertRaises(ProtocolError):
                client.fetch_snapshot_chunked(target)
            self.assertIsInstance(client.asked[-1], p.EndSnapshot)


if __name__ == "__main__":
    unittest.main()

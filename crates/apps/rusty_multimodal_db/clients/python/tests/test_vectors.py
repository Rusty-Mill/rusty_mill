"""ECO-FR-008 (i), offline: every line of tests/fixtures/wire-vectors.txt
at or below this client's declared protocol version decodes and
re-encodes byte-for-byte. No server, no Rust toolchain."""

import os
import sys
import unittest
import uuid

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))

from rusty_multimodal_db import protocol as p  # noqa: E402

FIXTURE = os.path.join(HERE, "..", "..", "..", "tests", "fixtures", "wire-vectors.txt")


def load_vectors():
    with open(FIXTURE, encoding="utf-8") as f:
        for line in f:
            line = line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            name, version, hexbytes = line.split("\t")
            yield name, int(version), bytes.fromhex(hexbytes)


class WireVectors(unittest.TestCase):
    def test_fixture_exists_and_is_not_empty(self):
        vectors = list(load_vectors())
        self.assertGreater(len(vectors), 40)
        self.assertTrue(any(n.startswith("Request/") for n, _, _ in vectors))
        self.assertTrue(any(n.startswith("Response/") for n, _, _ in vectors))

    def test_every_request_vector_round_trips(self):
        for name, version, data in load_vectors():
            if not name.startswith("Request/") or version > p.PROTOCOL_VERSION:
                continue
            with self.subTest(vector=name):
                value = p.decode_request(data)
                self.assertEqual(p.encode_request(value), data)

    def test_every_response_vector_round_trips(self):
        for name, version, data in load_vectors():
            if not name.startswith("Response/") or version > p.PROTOCOL_VERSION:
                continue
            with self.subTest(vector=name):
                value = p.decode_response(data)
                self.assertEqual(p.encode_response(value), data)

    def test_no_vector_is_above_the_declared_version(self):
        # The fixture and this client are pinned to the same version; a
        # newer fixture is a signal to update REQUEST_INTRODUCED_AT & co.
        for name, version, _ in load_vectors():
            with self.subTest(vector=name):
                self.assertLessEqual(version, p.PROTOCOL_VERSION)

    def test_handshake_frames_match_the_specification_examples(self):
        # SERVER-002 §4's worked examples: Hello { 21 } and GetById(uuid 1).
        import uuid

        self.assertEqual(p.encode_request(p.Hello(22)), bytes.fromhex("0a00000016000000"))
        self.assertEqual(
            p.frame(p.encode_request(p.GetById(uuid.UUID(int=1)))),
            bytes.fromhex("1c000000" "00000000" "1000000000000000" + "00" * 15 + "01"),
        )

    def test_mvcc_isolation_flag_is_8_and_the_declared_version_is_33(self):
        # MVCC2-FR-004/011, ADR-0072: real MVCC's BeginWith bit — this
        # client declares protocol 33, so it may send it (compatibility
        # rule 4), and the wire shape is BeginWith's existing plain u32
        # flags field, no new codec logic needed.
        self.assertEqual(p.SESSION_MVCC_ISOLATION, 8)
        self.assertEqual(p.PROTOCOL_VERSION, 34)
        req = p.BeginWith(p.SESSION_MVCC_ISOLATION)
        data = p.encode_request(req)
        self.assertEqual(p.decode_request(data), req)
        self.assertEqual(p.encode_request(p.decode_request(data)), data)
        # Matches the pinned fixture byte-for-byte.
        self.assertEqual(data, bytes.fromhex("0e00000008000000"))
        # Composes with the other three bits independently (ADR-0072).
        combined = p.BeginWith(
            p.SESSION_READ_YOUR_WRITES
            | p.SESSION_VALIDATE_ON_STAGE
            | p.SESSION_SNAPSHOT_ISOLATION
            | p.SESSION_MVCC_ISOLATION
        )
        combined_data = p.encode_request(combined)
        self.assertEqual(p.decode_request(combined_data), combined)
        self.assertEqual(combined_data, bytes.fromhex("0e0000000f000000"))


    def test_null_is_variant_6_and_decodes_to_none(self):
        # NUL-FR-001, ADR-0117 (protocol 31): a unit variant, the index
        # alone on the wire; scan_value_py reads it as None.
        record = p.Record(uuid.UUID(int=1), ((11, p.Null()),))
        data = p.encode_response(record)
        self.assertTrue(data.endswith(bytes.fromhex("0b00" "06000000")))
        self.assertEqual(p.decode_response(data), record)
        self.assertIsNone(p.scan_value_py(p.Null()))

    def test_describe_nullable_is_request_38_and_nullable_fields_is_response_25(self):
        # NLC-FR-005, ADR-0128 (protocol 32).
        self.assertEqual(p.encode_request(p.DescribeNullable()), bytes.fromhex("26000000"))
        reply = p.NullableFields([11, 12])
        data = p.encode_response(reply)
        self.assertEqual(data, bytes.fromhex("19000000" "0200000000000000" "0b00" "0c00"))
        self.assertEqual(p.decode_response(data), reply)
        self.assertEqual(p.REQUEST_INTRODUCED_AT[p.DescribeNullable], 32)

    def test_fetch_since_is_request_39_and_changes_snapshot_at_are_26_27(self):
        # RPL-FR-002, ADR-0131 (protocol 34).
        req = p.FetchSince(2, 5, 10)
        data = p.encode_request(req)
        self.assertEqual(
            data, bytes.fromhex("27000000" "0200000000000000" "0500000000000000" "0a000000")
        )
        self.assertEqual(p.decode_request(data), req)
        self.assertEqual(p.REQUEST_INTRODUCED_AT[p.FetchSince], 34)
        for reply in (
            p.Changes(2, 6, 9, ()),
            p.SnapshotAt((("a", b"\xff"),), 2, 5),
        ):
            self.assertEqual(p.decode_response(p.encode_response(reply)), reply)
        self.assertEqual(p.ErrorCode.Gone, 16)

    def test_write_op_update_field_is_5_and_write_result_updated_is_9(self):
        # TXS-FR-001, ADR-0130 (protocol 33).
        batch = p.WriteBatch((p.WoUpdateField(uuid.UUID(int=1), 10, p.I64(5)),), True)
        data = p.encode_request(batch)
        self.assertEqual(
            data,
            bytes.fromhex(
                "1f000000" "0100000000000000" "05000000" "1000000000000000" + "00" * 15 + "01"
                "0a00" "01000000" "0500000000000000" "01"
            ),
        )
        self.assertEqual(p.decode_request(data), batch)
        reply = p.BatchResults((p.WrUpdated(),))
        self.assertEqual(
            p.encode_response(reply), bytes.fromhex("14000000" "0100000000000000" "09000000")
        )


if __name__ == "__main__":
    unittest.main()

"""Executable checks for the synthetic memory-sync v1 draft fixtures.

This is deliberately a contract model, not either product's HTTP implementation.
"""

import copy
import datetime as dt
import json
import math
import pathlib
import re
import unittest


HERE = pathlib.Path(__file__).parent
CASES = json.loads((HERE / "fixtures" / "cases.json").read_text(encoding="utf-8"))
ID_RE = re.compile(r"[A-Za-z0-9_-]{1,128}\Z")
CLOSED_TYPES = {"episodic", "semantic", "procedural", "unclassified"}
REQUIRED = {
    "id", "content", "category", "tags", "source", "metadata", "created_at",
    "updated_at", "client", "node_id", "capture_id", "source_capture_id",
    "memory_type", "status", "deleted_at", "sensitive", "remind_at",
    "superseded_by", "subject", "predicate", "object", "accessed_at",
    "access_count", "decay_rate", "vitality", "base_weight",
}


class ContractError(ValueError):
    pass


def validate_envelope(envelope, supported_capabilities):
    required = {"contract", "version", "capabilities", "node_id", "body"}
    if not required.issubset(envelope):
        raise ContractError("invalid_envelope")
    if envelope["contract"] != "rusty-mill.memory-sync" or envelope["version"] != 1:
        raise ContractError("unsupported_version")
    if not isinstance(envelope["capabilities"], list) or not all(
        isinstance(item, str) for item in envelope["capabilities"]
    ):
        raise ContractError("invalid_envelope")
    return set(envelope["capabilities"]) & set(supported_capabilities)


def timestamp(value):
    if not isinstance(value, str) or not re.search(r"(?:Z|[+-]\d\d:\d\d)\Z", value):
        raise ContractError("invalid_timestamp")
    try:
        return dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise ContractError("invalid_timestamp") from error


def validate_record(record, capabilities):
    if set(record) != REQUIRED:
        raise ContractError("invalid_record_shape")
    if not isinstance(record["id"], str) or not ID_RE.fullmatch(record["id"]):
        raise ContractError("invalid_id")
    for field in ("created_at", "updated_at"):
        timestamp(record[field])
    for field in ("deleted_at", "remind_at", "accessed_at"):
        if record[field] is not None:
            timestamp(record[field])
    if "memory-types.closed" in capabilities and record["memory_type"] not in CLOSED_TYPES:
        raise ContractError("unsupported_memory_type")
    status_deleted = record["status"] == "deleted"
    dated_deleted = record["deleted_at"] is not None
    if status_deleted and "tombstone.status" not in capabilities:
        raise ContractError("tombstone_capability_mismatch")
    if dated_deleted and "tombstone.deleted-at" not in capabilities:
        raise ContractError("tombstone_capability_mismatch")
    if record["sensitive"] and "field.sensitive" not in capabilities:
        raise ContractError("unsupported_sensitive")
    if record["remind_at"] is not None and "field.remind-at" not in capabilities:
        raise ContractError("unsupported_reminder")
    lineage = ("capture_id", "source_capture_id", "superseded_by", "subject", "predicate", "object")
    if any(record[field] is not None for field in lineage) and "field.lineage" not in capabilities:
        raise ContractError("unsupported_lineage")
    if not isinstance(record["access_count"], int) or record["access_count"] < 0:
        raise ContractError("invalid_record_type")
    if not all(isinstance(record[field], (int, float)) and math.isfinite(record[field])
               for field in ("decay_rate", "vitality", "base_weight")):
        raise ContractError("invalid_record_type")


def validate_push_accounting(request_ids, response):
    outcomes = list(response["processed_ids"]) + [item["id"] for item in response["refused"]]
    if len(request_ids) != len(set(request_ids)) or sorted(outcomes) != sorted(request_ids):
        raise ContractError("incomplete_push_accounting")
    if len(outcomes) != len(set(outcomes)):
        raise ContractError("incomplete_push_accounting")


def validate_cursor(request_capability, negotiated_capabilities):
    if request_capability not in {"cursor.timestamp-id", "cursor.sequence"}:
        raise ContractError("cursor_capability_mismatch")
    if request_capability not in negotiated_capabilities:
        raise ContractError("cursor_capability_mismatch")


def resolve_equal_time(stored, incoming):
    if timestamp(stored["updated_at"]) != timestamp(incoming["updated_at"]):
        return "newer" if timestamp(incoming["updated_at"]) > timestamp(stored["updated_at"]) else "older"
    if json.dumps(stored, sort_keys=True, separators=(",", ":")) == json.dumps(incoming, sort_keys=True, separators=(",", ":")):
        return "idempotent"
    raise ContractError("equal_time_conflict")


class ContractFixtures(unittest.TestCase):
    def test_version_and_capability_negotiation(self):
        envelope = {
            "contract": "rusty-mill.memory-sync", "version": 1,
            "capabilities": ["cursor.timestamp-id", "memory-types.closed"],
            "node_id": "synthetic-node", "body": {},
        }
        self.assertEqual(
            validate_envelope(envelope, {"cursor.timestamp-id", "field.sensitive"}),
            {"cursor.timestamp-id"},
        )
        envelope["version"] = 2
        with self.assertRaisesRegex(ContractError, "^unsupported_version$"):
            validate_envelope(envelope, set())

    def test_compatible_record(self):
        validate_record(CASES["compatible"]["record"], set(CASES["compatible"]["capabilities"]))

    def test_negative_records(self):
        base = CASES["compatible"]
        for case in CASES["negative"]:
            with self.subTest(case=case["name"]):
                record = copy.deepcopy(base["record"])
                record.update(case["mutate"])
                capabilities = set(case.get("capabilities", base["capabilities"]))
                with self.assertRaisesRegex(ContractError, f"^{case['error']}$"):
                    validate_record(record, capabilities)

    def test_equal_time_conflict_and_idempotence(self):
        stored = copy.deepcopy(CASES["compatible"]["record"])
        self.assertEqual(resolve_equal_time(stored, copy.deepcopy(stored)), "idempotent")
        incoming = copy.deepcopy(stored)
        incoming["content"] = "Different meaning at the same instant"
        with self.assertRaisesRegex(ContractError, "^equal_time_conflict$"):
            resolve_equal_time(stored, incoming)

    def test_complete_per_id_accounting(self):
        validate_push_accounting(["a", "b"], {
            "processed_ids": ["a"], "refused": [{"id": "b", "code": "invalid_id"}]
        })
        with self.assertRaisesRegex(ContractError, "^incomplete_push_accounting$"):
            validate_push_accounting(["a", "b"], {"processed_ids": ["a"], "refused": []})
        with self.assertRaisesRegex(ContractError, "^incomplete_push_accounting$"):
            validate_push_accounting(["a"], {
                "processed_ids": ["a"], "refused": [{"id": "a", "code": "other"}]
            })

    def test_cursor_capability_mismatch(self):
        validate_cursor("cursor.timestamp-id", {"cursor.timestamp-id"})
        with self.assertRaisesRegex(ContractError, "^cursor_capability_mismatch$"):
            validate_cursor("cursor.sequence", {"cursor.timestamp-id"})


if __name__ == "__main__":
    unittest.main()

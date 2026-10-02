"""Executable checks for the synthetic memory-sync v1 draft fixtures.

This is deliberately a contract model, not either product's HTTP implementation.
"""

import copy
import datetime as dt
from decimal import Decimal
import json
import math
import pathlib
import re
import unittest


HERE = pathlib.Path(__file__).parent
CASES = json.loads(
    (HERE / "fixtures" / "cases.json").read_text(encoding="utf-8"),
    parse_float=Decimal,
)
ID_RE = re.compile(r"[A-Za-z0-9_-]{1,128}\Z")
TIMESTAMP_RE = re.compile(
    r"([0-9]{4})-([0-9]{2})-([0-9]{2})T([0-9]{2}):([0-9]{2}):([0-9]{2})"
    r"(?:\.([0-9]{1,9}))?(Z|[+-][0-9]{2}:[0-9]{2})\Z"
)
CLOSED_TYPES = {"episodic", "semantic", "procedural", "unclassified"}
MEMORY_TYPE_CAPABILITIES = {"memory-types.closed", "memory-types.open"}
CURSOR_CAPABILITIES = {"cursor.timestamp-id", "cursor.sequence"}
TIMESTAMP_FIELDS = {"created_at", "updated_at", "deleted_at", "remind_at", "accessed_at"}
REQUIRED = {
    "id", "content", "category", "tags", "source", "metadata", "created_at",
    "updated_at", "client", "node_id", "capture_id", "source_capture_id",
    "memory_type", "status", "deleted_at", "sensitive", "remind_at",
    "superseded_by", "subject", "predicate", "object", "accessed_at",
    "access_count", "decay_rate", "vitality", "base_weight",
}
NULLABLE_STRINGS = {
    "node_id", "capture_id", "source_capture_id", "superseded_by", "subject",
    "predicate", "object",
}


class ContractError(ValueError):
    pass


def is_integer(value):
    return isinstance(value, int) and not isinstance(value, bool)


def is_number(value):
    return isinstance(value, (int, float, Decimal)) and not isinstance(value, bool)


def is_finite_number(value):
    """Check supported exact numbers without coercing integers or decimals to float."""
    if isinstance(value, bool):
        return False
    if isinstance(value, int):
        return True
    if isinstance(value, Decimal):
        return value.is_finite()
    if isinstance(value, float):
        return math.isfinite(value)
    return False


def validate_capability_choice(capabilities, alternatives, error):
    if len(set(capabilities) & alternatives) != 1:
        raise ContractError(error)


def validate_envelope(envelope, supported_capabilities):
    required = {"contract", "version", "capabilities", "node_id", "body"}
    if not isinstance(envelope, dict) or not required.issubset(envelope):
        raise ContractError("invalid_envelope")
    if envelope["contract"] != "rusty-mill.memory-sync" or not is_integer(envelope["version"]):
        raise ContractError("invalid_envelope")
    if envelope["version"] != 1:
        raise ContractError("unsupported_version")
    capabilities = envelope["capabilities"]
    if (not isinstance(capabilities, list) or not all(isinstance(item, str) and item for item in capabilities)
            or len(capabilities) != len(set(capabilities))):
        raise ContractError("invalid_envelope")
    if (not isinstance(envelope["node_id"], str) or not envelope["node_id"]
            or not isinstance(envelope["body"], dict)):
        raise ContractError("invalid_envelope")
    validate_capability_choice(capabilities, MEMORY_TYPE_CAPABILITIES, "memory_type_capability_mismatch")
    return set(capabilities) & set(supported_capabilities)


def timestamp(value):
    """Return an exact UTC nanosecond key for the contract timestamp grammar."""
    if not isinstance(value, str):
        raise ContractError("invalid_timestamp")
    match = TIMESTAMP_RE.fullmatch(value)
    if match is None:
        raise ContractError("invalid_timestamp")
    year, month, day, hour, minute, second = map(int, match.groups()[:6])
    fraction, offset = match.group(7), match.group(8)
    try:
        date = dt.date(year, month, day)
    except ValueError as error:
        raise ContractError("invalid_timestamp") from error
    if hour > 23 or minute > 59 or second > 59:
        raise ContractError("invalid_timestamp")
    if offset == "Z":
        offset_seconds = 0
    else:
        offset_hour, offset_minute = map(int, offset[1:].split(":"))
        if offset_hour > 23 or offset_minute > 59:
            raise ContractError("invalid_timestamp")
        offset_seconds = (offset_hour * 60 + offset_minute) * 60
        if offset[0] == "-":
            offset_seconds = -offset_seconds
    days = date.toordinal() - dt.date(1970, 1, 1).toordinal()
    seconds = days * 86400 + hour * 3600 + minute * 60 + second - offset_seconds
    nanoseconds = int((fraction or "").ljust(9, "0")) if fraction else 0
    return seconds * 1_000_000_000 + nanoseconds


def validate_record(record, capabilities):
    if not isinstance(record, dict) or set(record) != REQUIRED:
        raise ContractError("invalid_record_shape")
    capabilities = set(capabilities)
    validate_capability_choice(capabilities, MEMORY_TYPE_CAPABILITIES, "memory_type_capability_mismatch")

    if not isinstance(record["id"], str) or not ID_RE.fullmatch(record["id"]):
        raise ContractError("invalid_id")
    for field in ("content", "category", "source", "client", "memory_type", "status"):
        if not isinstance(record[field], str) or not record[field]:
            raise ContractError("invalid_record_type")
    if not isinstance(record["tags"], list) or not all(isinstance(tag, str) for tag in record["tags"]):
        raise ContractError("invalid_record_type")
    if not isinstance(record["metadata"], dict):
        raise ContractError("invalid_record_type")
    if not all(record[field] is None or isinstance(record[field], str) for field in NULLABLE_STRINGS):
        raise ContractError("invalid_record_type")
    if record["superseded_by"] is not None and not ID_RE.fullmatch(record["superseded_by"]):
        raise ContractError("invalid_id")
    if not isinstance(record["sensitive"], bool):
        raise ContractError("invalid_record_type")
    if not is_integer(record["access_count"]) or record["access_count"] < 0:
        raise ContractError("invalid_record_type")
    if not all(is_finite_number(record[field])
               for field in ("decay_rate", "vitality", "base_weight")):
        raise ContractError("invalid_record_type")

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


def validate_push_body(body):
    if not isinstance(body, dict) or set(body) != {"records"} or not isinstance(body["records"], list):
        raise ContractError("invalid_push_shape")
    ids = []
    for record in body["records"]:
        if not isinstance(record, dict) or not isinstance(record.get("id"), str):
            raise ContractError("invalid_push_shape")
        ids.append(record["id"])
    if len(ids) != len(set(ids)):
        raise ContractError("duplicate_push_id")
    return ids


def validate_push_accounting(request_ids, response):
    if (not isinstance(response, dict) or set(response) != {"processed_ids", "refused"}
            or not isinstance(response["processed_ids"], list) or not isinstance(response["refused"], list)
            or not all(isinstance(item, str) for item in response["processed_ids"])):
        raise ContractError("invalid_push_accounting_shape")
    refused_ids = []
    for item in response["refused"]:
        if (not isinstance(item, dict) or not {"id", "code"}.issubset(item)
                or not set(item).issubset({"id", "code", "detail"})
                or not isinstance(item["id"], str) or not isinstance(item["code"], str)
                or not item["code"] or ("detail" in item and not isinstance(item["detail"], str))):
            raise ContractError("invalid_push_accounting_shape")
        refused_ids.append(item["id"])
    outcomes = response["processed_ids"] + refused_ids
    if (len(request_ids) != len(set(request_ids)) or sorted(outcomes) != sorted(request_ids)
            or len(outcomes) != len(set(outcomes))):
        raise ContractError("incomplete_push_accounting")


def validate_cursor(request_capability, negotiated_capabilities):
    if request_capability not in CURSOR_CAPABILITIES or request_capability not in negotiated_capabilities:
        raise ContractError("cursor_capability_mismatch")


def validate_pull_request(body, request_capability, negotiated_capabilities):
    validate_cursor(request_capability, negotiated_capabilities)
    if (not isinstance(body, dict) or set(body) != {"cursor", "limit"}
            or (body["cursor"] is not None and not isinstance(body["cursor"], str))
            or not is_integer(body["limit"]) or not 1 <= body["limit"] <= 1000):
        raise ContractError("invalid_pull_request")


def validate_pull_response(body):
    if (not isinstance(body, dict) or set(body) != {"records", "next_cursor", "has_more"}
            or not isinstance(body["records"], list) or not isinstance(body["has_more"], bool)):
        raise ContractError("invalid_pull_response")
    if body["has_more"]:
        if not isinstance(body["next_cursor"], str) or not body["next_cursor"]:
            raise ContractError("invalid_pull_response")
    elif body["next_cursor"] is not None:
        raise ContractError("invalid_pull_response")


def canonical_equal(left, right, timestamp_field=False, record_object=True):
    """Compare parsed JSON by v1's language-independent value rules."""
    if timestamp_field and left is not None and right is not None:
        return timestamp(left) == timestamp(right)
    if isinstance(left, bool) or isinstance(right, bool):
        return type(left) is type(right) and left == right
    if is_number(left) and is_number(right):
        return Decimal(str(left)) == Decimal(str(right))
    if isinstance(left, dict) and isinstance(right, dict):
        return set(left) == set(right) and all(
            canonical_equal(
                left[key], right[key],
                timestamp_field=record_object and key in TIMESTAMP_FIELDS,
                record_object=False,
            )
            for key in left
        )
    if isinstance(left, list) and isinstance(right, list):
        return len(left) == len(right) and all(
            canonical_equal(a, b, record_object=False) for a, b in zip(left, right)
        )
    return type(left) is type(right) and left == right


def resolve_record(stored, incoming):
    effective = copy.deepcopy(incoming)
    effective["created_at"] = stored["created_at"]
    stored_time, incoming_time = timestamp(stored["updated_at"]), timestamp(effective["updated_at"])
    if incoming_time > stored_time:
        return "newer", effective
    if incoming_time < stored_time:
        return "older", copy.deepcopy(stored)
    if canonical_equal(stored, effective):
        return "idempotent", copy.deepcopy(stored)
    raise ContractError("equal_time_conflict")


class ContractFixtures(unittest.TestCase):
    def test_version_and_capability_negotiation(self):
        envelope = {"contract": "rusty-mill.memory-sync", "version": 1,
                    "capabilities": ["cursor.timestamp-id", "memory-types.closed"],
                    "node_id": "synthetic-node", "body": {}}
        self.assertEqual(validate_envelope(envelope, {"cursor.timestamp-id", "field.sensitive"}),
                         {"cursor.timestamp-id"})
        for capabilities in ([], ["memory-types.open", "memory-types.closed"]):
            with self.subTest(capabilities=capabilities), self.assertRaisesRegex(
                    ContractError, "^memory_type_capability_mismatch$"):
                validate_envelope({**envelope, "capabilities": capabilities}, set(capabilities))
        with self.assertRaisesRegex(ContractError, "^unsupported_version$"):
            validate_envelope({**envelope, "version": 2}, set())
        for mutation in ({"version": True}, {"node_id": 17}, {"body": []}, {"capabilities": [17]}):
            with self.subTest(mutation=mutation), self.assertRaisesRegex(ContractError, "^invalid_envelope$"):
                validate_envelope({**envelope, **mutation}, set())

    def test_compatible_record_and_open_type(self):
        validate_record(CASES["compatible"]["record"], set(CASES["compatible"]["capabilities"]))
        record = {**CASES["compatible"]["record"], "memory_type": "fact"}
        capabilities = set(CASES["compatible"]["capabilities"]) - {"memory-types.closed"} | {"memory-types.open"}
        validate_record(record, capabilities)

    def test_negative_records(self):
        base = CASES["compatible"]
        for case in CASES["negative"]:
            with self.subTest(case=case["name"]):
                record = copy.deepcopy(base["record"]); record.update(case["mutate"])
                capabilities = set(case.get("capabilities", base["capabilities"]))
                with self.assertRaisesRegex(ContractError, f"^{case['error']}$"):
                    validate_record(record, capabilities)

    def test_strict_lossless_timestamps(self):
        for value in CASES["invalid_timestamps"]:
            with self.subTest(value=value), self.assertRaisesRegex(ContractError, "^invalid_timestamp$"):
                timestamp(value)
        self.assertEqual(timestamp("2026-01-02T03:04:05Z"), timestamp("2026-01-02T04:04:05+01:00"))
        self.assertLess(timestamp("2026-01-02T03:04:05.0000001Z"), timestamp("2026-01-02T03:04:05.0000002Z"))

    def test_conflict_stored_state_and_canonical_equality(self):
        stored = copy.deepcopy(CASES["compatible"]["record"])
        outcome, result = resolve_record(stored, copy.deepcopy(stored))
        self.assertEqual((outcome, result), ("idempotent", stored))
        equivalent = copy.deepcopy(stored); equivalent["updated_at"] = "2026-01-02T04:04:06+01:00"
        equivalent["vitality"] = 1
        self.assertEqual(resolve_record(stored, equivalent)[0], "idempotent")
        newer = copy.deepcopy(stored); newer.update(content="new", created_at="2025-01-01T00:00:00Z",
                                                     updated_at="2026-01-02T03:04:06.0000001Z")
        outcome, result = resolve_record(stored, newer)
        self.assertEqual(outcome, "newer"); self.assertEqual(result["content"], "new")
        self.assertEqual(result["created_at"], stored["created_at"])
        self.assertEqual(resolve_record(result, newer), ("idempotent", result))
        retry_conflict = copy.deepcopy(newer); retry_conflict["content"] = "genuinely different"
        with self.assertRaisesRegex(ContractError, "^equal_time_conflict$"):
            resolve_record(result, retry_conflict)
        older = copy.deepcopy(stored); older.update(content="old", updated_at="2026-01-02T03:04:05Z")
        self.assertEqual(resolve_record(stored, older), ("older", stored))
        conflict = copy.deepcopy(stored); conflict["content"] = "different"
        with self.assertRaisesRegex(ContractError, "^equal_time_conflict$"):
            resolve_record(stored, conflict)

    def test_exact_large_numbers_and_superseded_id(self):
        base = copy.deepcopy(CASES["compatible"]["record"])
        capabilities = set(CASES["compatible"]["capabilities"])
        base["decay_rate"] = CASES["exact_numbers"]["large_decimal"]
        base["vitality"] = CASES["exact_numbers"]["large_integer"]
        validate_record(base, capabilities)
        equivalent = copy.deepcopy(base); equivalent["decay_rate"] = 10 ** 400
        self.assertTrue(canonical_equal(base, equivalent))
        adjacent = copy.deepcopy(equivalent); adjacent["decay_rate"] += 1
        self.assertFalse(canonical_equal(base, adjacent))
        for value in (Decimal("NaN"), Decimal("Infinity"), float("nan"), float("inf"), True):
            invalid = copy.deepcopy(base); invalid["vitality"] = value
            with self.subTest(value=value), self.assertRaisesRegex(ContractError, "^invalid_record_type$"):
                validate_record(invalid, capabilities)
        base["superseded_by"] = "opaque_valid-ID"
        validate_record(base, capabilities)
        self.assertEqual(base["superseded_by"], "opaque_valid-ID")

    def test_push_shapes_and_complete_accounting(self):
        self.assertEqual(validate_push_body({"records": [{"id": "a"}, {"id": "b"}]}), ["a", "b"])
        validate_push_accounting(["a", "b"], {"processed_ids": ["a"],
                                                "refused": [{"id": "b", "code": "invalid_id"}]})
        bad = [
            ({"records": "not-an-array"}, "invalid_push_shape"),
            ({"records": [{"id": "a"}, {"id": "a"}]}, "duplicate_push_id"),
        ]
        for body, error in bad:
            with self.subTest(body=body), self.assertRaisesRegex(ContractError, f"^{error}$"):
                validate_push_body(body)
        for request_ids, response, error in [
            (["a", "b"], {"processed_ids": ["a"], "refused": []}, "incomplete_push_accounting"),
            (["a"], {"processed_ids": ["a"], "refused": [{"id": "a", "code": "other"}]}, "incomplete_push_accounting"),
            (["a"], {"processed_ids": "a", "refused": []}, "invalid_push_accounting_shape"),
            (["a"], {"processed_ids": [], "refused": [{"id": "a"}]}, "invalid_push_accounting_shape"),
        ]:
            with self.subTest(response=response), self.assertRaisesRegex(ContractError, f"^{error}$"):
                validate_push_accounting(request_ids, response)

    def test_pull_rules_and_cursor_capability(self):
        negotiated = {"cursor.timestamp-id"}
        validate_pull_request({"cursor": None, "limit": 1}, "cursor.timestamp-id", negotiated)
        validate_pull_request({"cursor": "opaque", "limit": 1000}, "cursor.timestamp-id", negotiated)
        validate_pull_response({"records": [], "next_cursor": "opaque", "has_more": True})
        validate_pull_response({"records": [], "next_cursor": None, "has_more": False})
        for body in ({"cursor": None, "limit": 0}, {"cursor": None, "limit": 1001},
                     {"cursor": None, "limit": True}, {"cursor": 3, "limit": 10}):
            with self.subTest(body=body), self.assertRaisesRegex(ContractError, "^invalid_pull_request$"):
                validate_pull_request(body, "cursor.timestamp-id", negotiated)
        for body in ({"records": [], "next_cursor": None, "has_more": True},
                     {"records": [], "next_cursor": "stale", "has_more": False},
                     {"records": {}, "next_cursor": None, "has_more": False}):
            with self.subTest(body=body), self.assertRaisesRegex(ContractError, "^invalid_pull_response$"):
                validate_pull_response(body)
        with self.assertRaisesRegex(ContractError, "^cursor_capability_mismatch$"):
            validate_pull_request({"cursor": None, "limit": 10}, "cursor.sequence", negotiated)


if __name__ == "__main__":
    unittest.main()

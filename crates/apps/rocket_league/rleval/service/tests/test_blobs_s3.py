"""S3 blob backend (§5): the public blobs.* API over object storage.

Uses moto — a real boto3 client against a faked in-memory S3 — so no MinIO/AWS is
needed and the adapter's actual boto3 calls are exercised.
"""

from __future__ import annotations

import io

import pytest

pytest.importorskip("boto3")
pytest.importorskip("moto")

import boto3  # noqa: E402
from moto import mock_aws  # noqa: E402

from app import blobs, config  # noqa: E402

BUCKET = "rls-test-blobs"


def _client():
    return boto3.client("s3", region_name="us-east-1")


@pytest.fixture
def s3_blobs(monkeypatch):
    for k in ("AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "AWS_SESSION_TOKEN"):
        monkeypatch.setenv(k, "testing")
    monkeypatch.setenv("AWS_DEFAULT_REGION", "us-east-1")
    with mock_aws():
        _client().create_bucket(Bucket=BUCKET)
        monkeypatch.setattr(config.settings, "blob_backend", "s3")
        monkeypatch.setattr(config.settings, "s3_bucket", BUCKET)
        monkeypatch.setattr(config.settings, "s3_prefix", "artifacts/")
        monkeypatch.setattr(config.settings, "s3_region", "us-east-1")
        monkeypatch.setattr(config.settings, "s3_endpoint_url", None)
        blobs.reset_backend()  # build the S3 store inside the moto context
        try:
            yield
        finally:
            blobs.reset_backend()


def test_raw_blob_round_trips_on_s3(s3_blobs):
    assert not blobs.exists("rid1")  # exercises the head_object 404 path
    blobs.store("rid1", b"hello-s3")
    assert blobs.exists("rid1")
    assert blobs.load("rid1") == b"hello-s3"


def test_html_pdf_canonical_round_trip_on_s3(s3_blobs):
    blobs.store_html("rid2", "<h1>hi</h1>")
    assert blobs.has_html("rid2") and blobs.load_html("rid2") == "<h1>hi</h1>"

    blobs.store_pdf("rid2", b"%PDF-1.4 data")
    assert blobs.has_pdf("rid2") and blobs.load_pdf("rid2") == b"%PDF-1.4 data"
    blobs.delete_pdf("rid2")
    assert not blobs.has_pdf("rid2")

    blobs.store_canonical("rid2", b'{"k":1}')
    assert blobs.has_canonical("rid2") and blobs.load_canonical("rid2") == b'{"k":1}'


def test_prefix_is_applied_to_object_keys(s3_blobs):
    blobs.store("rid3", b"x")
    keys = [o["Key"] for o in _client().list_objects_v2(Bucket=BUCKET).get("Contents", [])]
    assert keys == ["artifacts/rid3"]


def test_encrypted_at_rest_on_s3(s3_blobs, monkeypatch):
    monkeypatch.setattr(config.settings, "encryption_key", "s3-key")
    marker = b"PLAINTEXT-S3-MARKER"
    blobs.store("rid4", marker + b"\x00" * 32)
    raw = _client().get_object(Bucket=BUCKET, Key="artifacts/rid4")["Body"].read()
    assert marker not in raw and raw[:4] == b"RLS1"       # the object is sealed
    assert blobs.load("rid4") == marker + b"\x00" * 32    # load transparently decrypts


def test_full_upload_pipeline_lands_artifacts_in_s3(monkeypatch, client, make_account):
    """End-to-end: upload → inline score → the raw blob + report artifacts in S3."""
    for k in ("AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "AWS_SESSION_TOKEN"):
        monkeypatch.setenv(k, "testing")
    monkeypatch.setenv("AWS_DEFAULT_REGION", "us-east-1")
    headers = {"X-Account-Email": "u@example.com"}
    with mock_aws():
        _client().create_bucket(Bucket=BUCKET)
        monkeypatch.setattr(config.settings, "blob_backend", "s3")
        monkeypatch.setattr(config.settings, "s3_bucket", BUCKET)
        monkeypatch.setattr(config.settings, "s3_prefix", "")
        monkeypatch.setattr(config.settings, "s3_region", "us-east-1")
        monkeypatch.setattr(config.settings, "s3_endpoint_url", None)
        blobs.reset_backend()
        try:
            make_account(credits_n=5)
            files = {"file": ("r.replay", io.BytesIO(b"s3-pipeline"), "application/octet-stream")}
            r = client.post("/v1/replays", files=files, headers=headers)
            assert r.status_code == 202
            rid = r.json()["replay_id"]
            assert client.get(f"/v1/replays/{rid}", headers=headers).json()["status"] == "done"

            keys = {o["Key"] for o in _client().list_objects_v2(Bucket=BUCKET).get("Contents", [])}
            assert rid in keys and f"{rid}.html" in keys  # raw blob + report HTML in S3
        finally:
            blobs.reset_backend()

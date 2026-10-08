"""Purchase/entitlement webhook (spec §8): grants, idempotency, provisioning."""

from __future__ import annotations

import hashlib
import hmac
import io
import json

from app import config


def _signed(payload: dict, secret: str):
    raw = json.dumps(payload).encode()
    sig = hmac.new(secret.encode(), raw, hashlib.sha256).hexdigest()
    return raw, {"X-Signature": sig, "content-type": "application/json"}


def _file(content: bytes = b"a-fake-replay"):
    return {"file": ("m.replay", io.BytesIO(content), "application/octet-stream")}


def _purchase(client, **kw):
    body = {"email": "b@example.com", "event_id": "evt_1", "kind": "monthly"}
    body.update(kw)
    return client.post("/internal/webhooks/purchase", json=body)


def test_monthly_grant_sets_book_and_credits(client):
    body = _purchase(client, grant_book=True).json()
    assert body["owns_book"] is True
    assert body["balance"] == 20  # settings.monthly_grant default


def test_idempotent_by_event_id(client):
    _purchase(client, grant_book=True)
    redelivered = _purchase(client, grant_book=True).json()  # same event_id
    assert redelivered["balance"] == 20  # not doubled to 40


def test_topup_adds_ten_after_monthly(client):
    _purchase(client, event_id="m1", grant_book=True)
    body = _purchase(client, event_id="t1", kind="topup").json()
    assert body["balance"] == 30


def test_valid_signature_is_accepted(client, monkeypatch):
    monkeypatch.setattr(config.settings, "webhook_secret", "shh")
    raw, headers = _signed(
        {"email": "b@example.com", "event_id": "s1", "kind": "monthly", "grant_book": True},
        "shh",
    )
    r = client.post("/internal/webhooks/purchase", content=raw, headers=headers)
    assert r.status_code == 200 and r.json()["owns_book"] is True


def test_bad_signature_is_rejected(client, monkeypatch):
    monkeypatch.setattr(config.settings, "webhook_secret", "shh")
    raw, _ = _signed({"email": "b@example.com", "event_id": "s1"}, "shh")
    r = client.post(
        "/internal/webhooks/purchase",
        content=raw,
        headers={"X-Signature": "deadbeef", "content-type": "application/json"},
    )
    assert r.status_code == 401


def test_missing_signature_is_rejected_when_secret_set(client, monkeypatch):
    monkeypatch.setattr(config.settings, "webhook_secret", "shh")
    raw, _ = _signed({"email": "b@example.com", "event_id": "s1"}, "shh")
    r = client.post(
        "/internal/webhooks/purchase",
        content=raw,
        headers={"content-type": "application/json"},
    )
    assert r.status_code == 401


def test_webhook_provisions_an_uploadable_account(client):
    _purchase(client, email="buyer@example.com", event_id="p1", grant_book=True)
    h = {"X-Account-Email": "buyer@example.com"}

    up = client.post("/v1/replays", files=_file(), headers=h)
    assert up.status_code == 202
    rid = up.json()["replay_id"]
    assert client.get(f"/v1/replays/{rid}", headers=h).json()["status"] == "done"
    # 20 granted, 1 spent.
    assert client.get("/v1/account/credits", headers=h).json()["balance"] == 19

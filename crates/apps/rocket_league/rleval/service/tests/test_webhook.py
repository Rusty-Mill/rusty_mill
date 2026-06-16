"""Purchase/entitlement webhook (spec §8): grants, idempotency, provisioning."""

from __future__ import annotations

import io


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


def test_webhook_provisions_an_uploadable_account(client):
    _purchase(client, email="buyer@example.com", event_id="p1", grant_book=True)
    h = {"X-Account-Email": "buyer@example.com"}

    up = client.post("/v1/replays", files=_file(), headers=h)
    assert up.status_code == 202
    rid = up.json()["replay_id"]
    assert client.get(f"/v1/replays/{rid}", headers=h).json()["status"] == "done"
    # 20 granted, 1 spent.
    assert client.get("/v1/account/credits", headers=h).json()["balance"] == 19

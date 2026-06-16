"""HTTP surface (spec §7): guards, the upload→score→report flow, dedupe, refund."""

from __future__ import annotations

import io

from app import config

H = {"X-Account-Email": "u@example.com"}


def _file(content: bytes = b"a-fake-replay"):
    return {"file": ("m.replay", io.BytesIO(content), "application/octet-stream")}


def test_healthz(client):
    assert client.get("/healthz").json() == {"ok": True}


def test_upload_requires_owns_book(client, make_account):
    make_account(owns_book=False, credits_n=5)
    r = client.post("/v1/replays", files=_file(), headers=H)
    assert r.status_code == 403


def test_upload_requires_credits(client, make_account):
    make_account(owns_book=True, credits_n=0)
    r = client.post("/v1/replays", files=_file(), headers=H)
    assert r.status_code == 402


def test_upload_scores_and_returns_report(client, make_account):
    make_account(owns_book=True, credits_n=3)
    r = client.post(
        "/v1/replays", files=_file(), data={"playlist": "ranked-2v2"}, headers=H
    )
    assert r.status_code == 202
    body = r.json()
    assert body["deduped"] is False
    rid = body["replay_id"]

    # The background task runs to completion within the request cycle.
    status = client.get(f"/v1/replays/{rid}", headers=H).json()
    assert status["status"] == "done"
    assert status["parser_version"] == "boxcars-fake"

    report = client.get(f"/v1/reports/{rid}", headers=H).json()
    assert report["status"] == "done"
    assert report["reports"][0]["player_id"] == "Alice"
    assert report["reports"][0]["composite"] == 72.0

    # One credit held then confirmed.
    assert client.get("/v1/account/credits", headers=H).json()["balance"] == 2


def test_reupload_is_deduped_and_charges_once(client, make_account):
    make_account(credits_n=3)
    client.post("/v1/replays", files=_file(b"same-bytes"), headers=H)
    r2 = client.post("/v1/replays", files=_file(b"same-bytes"), headers=H)
    assert r2.json()["deduped"] is True
    assert client.get("/v1/account/credits", headers=H).json()["balance"] == 2


def test_failed_scoring_auto_refunds(failing_client, make_account):
    make_account(credits_n=3)
    rid = failing_client.post("/v1/replays", files=_file(), headers=H).json()["replay_id"]
    status = failing_client.get(f"/v1/replays/{rid}", headers=H).json()
    assert status["status"] == "failed"
    # Refunded back to the starting balance.
    assert failing_client.get("/v1/account/credits", headers=H).json()["balance"] == 3


def test_upload_is_rate_limited_per_account(client, make_account, monkeypatch):
    monkeypatch.setattr(config.settings, "upload_rate_limit", 2)
    make_account(credits_n=10)
    for i in range(2):
        r = client.post("/v1/replays", files=_file(f"replay{i}".encode()), headers=H)
        assert r.status_code == 202
    # The third distinct upload within the window is throttled.
    assert client.post("/v1/replays", files=_file(b"replay2"), headers=H).status_code == 429


def test_artifacts_are_encrypted_at_rest(client, make_account, monkeypatch):
    from app import blobs

    monkeypatch.setattr(config.settings, "encryption_key", "at-rest-test-key")
    make_account(credits_n=5)
    marker = b"PLAINTEXT-REPLAY-MARKER"
    r = client.post("/v1/replays", files=_file(marker + b"\x00" * 64), headers=H)
    assert r.status_code == 202
    rid = r.json()["replay_id"]
    # On disk the raw blob is sealed (no plaintext marker), but load() decrypts it,
    # and the sealed HTML artifact round-trips through the scoring pipeline too.
    assert marker not in blobs._path(rid).read_bytes()
    assert blobs.load(rid) == marker + b"\x00" * 64
    assert "fake report" in blobs.load_html(rid)


def test_lock_profile(client, make_account):
    make_account(credits_n=1)
    r = client.post("/v1/account/lock-profile", json={"player_id": "Alice"}, headers=H)
    assert r.json()["locked_player_id"] == "Alice"
    assert client.get("/v1/account/credits", headers=H).json()["locked_player_id"] == "Alice"


def test_cannot_read_another_accounts_replay(client, make_account):
    make_account(email="owner@example.com", credits_n=2)
    rid = (
        client.post(
            "/v1/replays", files=_file(), headers={"X-Account-Email": "owner@example.com"}
        )
        .json()["replay_id"]
    )
    r = client.get(
        f"/v1/replays/{rid}", headers={"X-Account-Email": "intruder@example.com"}
    )
    assert r.status_code == 404

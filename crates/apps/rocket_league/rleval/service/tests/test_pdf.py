"""Report PDF endpoint (spec §4.10): render, cache, owner-gate, readiness."""

from __future__ import annotations

import io

from app.models import Replay

H = {"X-Account-Email": "u@example.com"}


def _file(content: bytes = b"a-fake-replay"):
    return {"file": ("m.replay", io.BytesIO(content), "application/octet-stream")}


def _upload(client, headers=H) -> str:
    return client.post("/v1/replays", files=_file(), headers=headers).json()["replay_id"]


def test_pdf_is_rendered_and_served(client, make_account):
    make_account(credits_n=2)
    rid = _upload(client)
    r = client.get(f"/v1/reports/{rid}/pdf", headers=H)
    assert r.status_code == 200
    assert r.headers["content-type"] == "application/pdf"
    assert r.content.startswith(b"%PDF")


def test_pdf_is_cached_and_rendered_once(client, make_account, pdf_renderer):
    make_account(credits_n=2)
    rid = _upload(client)
    client.get(f"/v1/reports/{rid}/pdf", headers=H)
    client.get(f"/v1/reports/{rid}/pdf", headers=H)
    assert pdf_renderer.calls == 1  # second request served from the cache


def test_pdf_is_owner_gated(client, make_account):
    make_account(email="owner@example.com", credits_n=2)
    rid = _upload(client, headers={"X-Account-Email": "owner@example.com"})
    r = client.get(
        f"/v1/reports/{rid}/pdf", headers={"X-Account-Email": "intruder@example.com"}
    )
    assert r.status_code == 404


def test_pdf_not_ready_returns_409(client, make_account, session):
    account = make_account(credits_n=2)
    session.add(
        Replay(id="queued1", account_id=account.id, playlist="x", status="queued")
    )
    session.commit()
    r = client.get("/v1/reports/queued1/pdf", headers=H)
    assert r.status_code == 409


def test_pdf_per_player_scopes_and_renders_on_demand(client, make_account, pdf_renderer):
    make_account(credits_n=2)
    rid = _upload(client)
    r = client.get(f"/v1/reports/{rid}/pdf?player=Alice", headers=H)
    assert r.status_code == 200
    assert r.content.startswith(b"%PDF")
    # The fake scorer echoes the player into the HTML, which the fake renderer
    # folds into the PDF bytes — proof the report was scoped to Alice.
    assert b"Alice" in r.content
    # Per-player is rendered on demand (not cached): a repeat re-renders.
    client.get(f"/v1/reports/{rid}/pdf?player=Alice", headers=H)
    assert pdf_renderer.calls == 2


def test_pdf_per_player_unknown_player_404(client, make_account):
    make_account(credits_n=2)
    rid = _upload(client)
    r = client.get(f"/v1/reports/{rid}/pdf?player=Nobody", headers=H)
    assert r.status_code == 404


def test_pdf_per_player_requires_canonical(client, make_account, scorer):
    scorer.canonical_blob = None  # nothing cached → can't scope per player
    make_account(credits_n=2)
    rid = _upload(client)
    r = client.get(f"/v1/reports/{rid}/pdf?player=Alice", headers=H)
    assert r.status_code == 409

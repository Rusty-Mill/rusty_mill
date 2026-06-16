"""Admin re-score (spec §8/§9): replace reports, refresh leaderboard + PDF cache."""

from __future__ import annotations

import io

from app.models import Replay
from app.scoring import PlayerScore

H = {"X-Account-Email": "u@example.com"}


def _file(content: bytes = b"a-fake-replay"):
    return {"file": ("m.replay", io.BytesIO(content), "application/octet-stream")}


def _upload(client) -> str:
    return client.post("/v1/replays", files=_file(), headers=H).json()["replay_id"]


def _newer_scorer(scorer, composite, config_version="score-v2"):
    """Mutate the in-process scorer to act like a newer worker version."""
    scorer.players = [
        PlayerScore(
            "Alice", composite, composite, composite, composite,
            "Champion", "Anchor", "challenge_timing", "x", "ok", "[]",
        )
    ]
    scorer.config_version = config_version


def test_rescore_replaces_reports_at_new_version(client, make_account, scorer):
    make_account(credits_n=2)
    rid = _upload(client)
    assert client.get(f"/v1/reports/{rid}", headers=H).json()["reports"][0]["composite"] == 72.0

    _newer_scorer(scorer, 90.0)
    res = client.post(f"/internal/rescore/{rid}").json()
    assert res["reports"] == 1 and res["score_config_version"] == "score-v2"

    reports = client.get(f"/v1/reports/{rid}", headers=H).json()["reports"]
    assert len(reports) == 1  # replaced, not appended
    assert reports[0]["composite"] == 90.0
    assert reports[0]["score_config_version"] == "score-v2"


def test_rescore_refreshes_the_leaderboard(client, session, make_account, scorer):
    account = make_account(credits_n=2)
    account.locked_player_id = "Alice"
    session.add(account)
    session.commit()

    rid = _upload(client)
    assert client.get("/v1/leaderboard").json()["entries"][0]["composite"] == 72.0

    _newer_scorer(scorer, 88.0)
    client.post(f"/internal/rescore/{rid}")
    assert client.get("/v1/leaderboard").json()["entries"][0]["composite"] == 88.0


def test_rescore_invalidates_the_pdf_cache(client, make_account, pdf_renderer):
    make_account(credits_n=2)
    rid = _upload(client)
    client.get(f"/v1/reports/{rid}/pdf", headers=H)
    assert pdf_renderer.calls == 1  # rendered + cached

    client.post(f"/internal/rescore/{rid}")
    client.get(f"/v1/reports/{rid}/pdf", headers=H)
    assert pdf_renderer.calls == 2  # cache dropped -> re-rendered


def test_rescore_missing_is_404(client, make_account):
    make_account(credits_n=1)
    assert client.post("/internal/rescore/nope").status_code == 404


def test_rescore_not_done_is_409(client, make_account, session):
    account = make_account(credits_n=1)
    session.add(Replay(id="q1", account_id=account.id, playlist="x", status="queued"))
    session.commit()
    assert client.post("/internal/rescore/q1").status_code == 409

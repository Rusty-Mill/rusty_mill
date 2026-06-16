"""Leaderboard materialization + the public feed (spec §5/§7/§8)."""

from __future__ import annotations

from sqlmodel import select

from app import leaderboard, service
from app.models import LeaderboardEntry, Replay, Report

H = {"X-Account-Email": "u@example.com"}


def _lock(session, account, player_id):
    account.locked_player_id = player_id
    session.add(account)
    session.commit()


def _report(session, replay_id, account_id, player_id, composite, confidence="ok"):
    if session.get(Replay, replay_id) is None:
        session.add(
            Replay(id=replay_id, account_id=account_id, playlist="x", status="done")
        )
    r = Report(
        replay_id=replay_id,
        player_id=player_id,
        composite=composite,
        first_man=composite,
        second_man=composite,
        general=composite,
        licence="Gold",
        player_type="Anchor",
        main_leak="overcommit_rate",
        focus_chapter="x",
        confidence=confidence,
        score_config_version="v",
        parser_version="v",
        metrics_json="[]",
    )
    session.add(r)
    session.commit()
    session.refresh(r)
    return r


def test_no_entry_without_a_locked_profile(session, make_account):
    acc = make_account(credits_n=0)  # owns_book, but no locked profile
    _report(session, "r1", acc.id, "Alice", 70.0)
    assert leaderboard.materialize(session, acc, "r1") is None
    assert session.exec(select(LeaderboardEntry)).first() is None


def test_low_confidence_is_ineligible(session, make_account):
    acc = make_account(credits_n=0)
    _lock(session, acc, "Alice")
    _report(session, "r1", acc.id, "Alice", 70.0, confidence="low_confidence")
    assert leaderboard.materialize(session, acc, "r1") is None


def test_only_the_locked_player_counts(session, make_account):
    acc = make_account(credits_n=0)
    _lock(session, acc, "Bob")
    _report(session, "r1", acc.id, "Alice", 90.0)  # not the locked profile
    assert leaderboard.materialize(session, acc, "r1") is None


def test_keeps_best_composite_per_season(session, make_account):
    acc = make_account(credits_n=0)
    _lock(session, acc, "Alice")

    _report(session, "r1", acc.id, "Alice", 60.0)
    assert leaderboard.materialize(session, acc, "r1").composite == 60.0
    session.commit()

    _report(session, "r2", acc.id, "Alice", 75.0)  # better -> replaces
    assert leaderboard.materialize(session, acc, "r2").composite == 75.0
    session.commit()

    _report(session, "r3", acc.id, "Alice", 50.0)  # worse -> ignored
    assert leaderboard.materialize(session, acc, "r3").composite == 75.0
    session.commit()

    rows = session.exec(
        select(LeaderboardEntry).where(LeaderboardEntry.account_id == acc.id)
    ).all()
    assert len(rows) == 1  # exactly one best-per-account-per-season


def test_successful_score_populates_the_leaderboard(session, make_account, scorer):
    acc = make_account(credits_n=2)
    _lock(session, acc, "Alice")  # the fake scorer reports player "Alice"
    replay = service.ingest(session, acc, b"flow", "ranked-2v2")
    service.run_scoring(session, replay, b"flow", scorer)

    entry = session.exec(
        select(LeaderboardEntry).where(LeaderboardEntry.account_id == acc.id)
    ).first()
    assert entry is not None
    assert entry.player_id == "Alice" and entry.composite == 72.0


def test_public_feed_is_sorted_by_composite(client, session, make_account):
    a = make_account(email="a@example.com")
    _lock(session, a, "Alice")
    b = make_account(email="b@example.com")
    _lock(session, b, "Bob")
    _report(session, "ra", a.id, "Alice", 70.0)
    leaderboard.materialize(session, a, "ra")
    _report(session, "rb", b.id, "Bob", 85.0)
    leaderboard.materialize(session, b, "rb")
    session.commit()

    res = client.get("/v1/leaderboard").json()
    assert [e["player_id"] for e in res["entries"]] == ["Bob", "Alice"]
    assert res["entries"][0]["rank"] == 1
    assert "uploaded_at" in res["entries"][0]

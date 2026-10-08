"""Leaderboard materialization + the public feed (spec §5/§7/§8/§13)."""

from __future__ import annotations

from datetime import datetime, timezone

from sqlmodel import select

from app import config, leaderboard, service
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
    assert res["closed"] is False  # current season


# --- Founding-N (§13) ---
def test_founding_numbers_assigned_in_order_up_to_cap(session, make_account, monkeypatch):
    monkeypatch.setattr(config.settings, "founding_n", 2)
    accts = []
    for i in range(3):
        a = make_account(email=f"f{i}@example.com")
        _lock(session, a, f"P{i}")
        _report(session, f"rf{i}", a.id, f"P{i}", 70.0)
        leaderboard.materialize(session, a, f"rf{i}")
        session.commit()
        accts.append(a)
    assert [a.founding_number for a in accts] == [1, 2, None]  # 3rd is past the cap


def test_founding_granted_once_not_re_numbered(session, make_account):
    a = make_account(email="once@example.com")
    _lock(session, a, "Alice")
    _report(session, "r1", a.id, "Alice", 60.0)
    leaderboard.materialize(session, a, "r1")
    _report(session, "r2", a.id, "Alice", 90.0)  # qualifies again
    leaderboard.materialize(session, a, "r2")
    session.commit()
    assert a.founding_number == 1


def test_feed_exposes_founding_number(client, session, make_account):
    a = make_account(email="ff@example.com")
    _lock(session, a, "Founder")
    _report(session, "rff", a.id, "Founder", 80.0)
    leaderboard.materialize(session, a, "rff")
    session.commit()
    assert client.get("/v1/leaderboard").json()["entries"][0]["founding_number"] == 1


# --- Recent feed (§13) ---
def test_recent_feed_is_newest_first(client, session, make_account):
    a = make_account(email="ra@example.com")
    _lock(session, a, "A")
    b = make_account(email="rb@example.com")
    _lock(session, b, "B")
    ra = _report(session, "r_a", a.id, "A", 60.0)
    ra.created_at = datetime(2026, 1, 1, 12, 0)  # older personal best
    session.add(ra); session.commit()
    leaderboard.materialize(session, a, "r_a")
    rb = _report(session, "r_b", b.id, "B", 50.0)
    rb.created_at = datetime(2026, 1, 2, 12, 0)  # newer, lower composite
    session.add(rb); session.commit()
    leaderboard.materialize(session, b, "r_b")
    session.commit()

    # By recency B leads (newer), even though A's composite is higher.
    res = client.get("/v1/leaderboard/recent").json()
    assert [e["player_id"] for e in res["entries"]] == ["B", "A"]


# --- Season close policy (§13) ---
def test_season_bounds_and_is_closed():
    assert leaderboard.season_bounds("2026-S1") == (
        datetime(2026, 1, 1, tzinfo=timezone.utc),
        datetime(2026, 4, 1, tzinfo=timezone.utc),
    )
    assert leaderboard.season_bounds("2025-S4")[1] == datetime(2026, 1, 1, tzinfo=timezone.utc)
    assert leaderboard.is_closed("2026-S1", now=datetime(2026, 4, 1, tzinfo=timezone.utc))
    assert not leaderboard.is_closed("2026-S1", now=datetime(2026, 3, 31, tzinfo=timezone.utc))


def test_feed_reports_closed_for_past_season(client):
    res = client.get("/v1/leaderboard?season=2020-S1").json()
    assert res["season"] == "2020-S1" and res["closed"] is True


def test_seasons_endpoint_lists_present_seasons(client, session, make_account):
    a = make_account(email="se@example.com")
    _lock(session, a, "S")
    _report(session, "rse", a.id, "S", 70.0)
    leaderboard.materialize(session, a, "rse")
    session.commit()
    res = client.get("/v1/leaderboard/seasons").json()
    assert any(s["season"] == leaderboard.current_season() for s in res["seasons"])

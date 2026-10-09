"""Credit-ledger logic (spec §8): balance, spend lifecycle, expiry."""

from __future__ import annotations

from datetime import timedelta

from app import credits
from app.models import Account, utcnow


def _account(session) -> Account:
    a = Account(email="c@example.com")
    session.add(a)
    session.commit()
    session.refresh(a)
    return a


def test_balance_sums_only_nonexpired(session):
    a = _account(session)
    credits.grant(session, a.id, 20, "monthly_grant")
    credits.grant(session, a.id, 10, "topup", expires_at=utcnow() + timedelta(days=30))
    credits.grant(session, a.id, 5, "topup", expires_at=utcnow() - timedelta(days=1))
    session.commit()
    assert credits.balance(session, a.id) == 30  # the expired +5 is excluded


def test_hold_then_confirm_consumes_one(session):
    a = _account(session)
    credits.grant(session, a.id, 3, "monthly_grant")
    session.commit()
    held = credits.hold(session, a.id, "r1")
    session.commit()
    assert held is not None and held.status == "pending"
    assert credits.balance(session, a.id) == 2
    credits.confirm(session, a.id, "r1")
    session.commit()
    assert credits.balance(session, a.id) == 2  # the spend persists


def test_refund_is_net_zero_and_idempotent(session):
    a = _account(session)
    credits.grant(session, a.id, 3, "monthly_grant")
    session.commit()
    credits.hold(session, a.id, "r1")
    session.commit()
    assert credits.balance(session, a.id) == 2
    credits.refund(session, a.id, "r1")
    session.commit()
    assert credits.balance(session, a.id) == 3  # net-zero vs. before the hold
    credits.refund(session, a.id, "r1")  # second refund is a no-op
    session.commit()
    assert credits.balance(session, a.id) == 3


def test_hold_is_idempotent_and_blocks_when_empty(session):
    a = _account(session)
    credits.grant(session, a.id, 1, "monthly_grant")
    session.commit()
    h1 = credits.hold(session, a.id, "r1")
    h2 = credits.hold(session, a.id, "r1")  # same replay -> same hold
    session.commit()
    assert h1 is not None and h1.id == h2.id
    assert credits.balance(session, a.id) == 0
    assert credits.hold(session, a.id, "r2") is None  # no balance for a new replay


def test_spend_expires_with_its_grant(session):
    # A hold inherits the grant's expiry, so an expired monthly grant leaves no
    # negative drift (no rollover) — both the +grant and the -spend drop off.
    a = _account(session)
    end = utcnow() + timedelta(days=30)
    credits.grant(session, a.id, 5, "monthly_grant", expires_at=end)
    session.commit()
    credits.hold(session, a.id, "r1")
    session.commit()
    after = end + timedelta(seconds=1)
    assert credits.balance(session, a.id, now=after) == 0

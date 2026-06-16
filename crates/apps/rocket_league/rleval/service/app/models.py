"""SQLModel tables (spec §5).

Idempotency hinges on `Replay.id = sha256(blob)` (§9). The credit ledger is
append-only; balance is a function of its non-expired rows (§8). Two fields are
added to the spec's `CreditLedger` sketch — `replay_id` and `status` — because §8
requires the spend lifecycle to be *idempotent by replay_id* and to tag the
soft-hold `pending` until it confirms or refunds.
"""

from __future__ import annotations

from datetime import datetime, timezone

from sqlmodel import Field, SQLModel


def utcnow() -> datetime:
    """Timezone-aware UTC now (default for created/updated timestamps)."""
    return datetime.now(timezone.utc)


class Account(SQLModel, table=True):
    id: int | None = Field(default=None, primary_key=True)
    email: str = Field(index=True, unique=True)
    owns_book: bool = False  # entitlement gate, set by the purchase webhook (§8)
    locked_player_id: str | None = None  # the locked leaderboard profile (§5)
    created_at: datetime = Field(default_factory=utcnow)


class CreditLedger(SQLModel, table=True):
    id: int | None = Field(default=None, primary_key=True)
    account_id: int = Field(index=True, foreign_key="account.id")
    delta: int  # +20 monthly, +10 top-up, -1 spend, +1 refund
    reason: str  # "monthly_grant" | "topup" | "spend" | "refund"
    # Ties a spend/refund to the replay it paid for, so the lifecycle is
    # idempotent (§8) — never hold or refund the same replay twice.
    replay_id: str | None = Field(default=None, index=True)
    status: str = "settled"  # "pending" (soft-hold) | "settled"
    expires_at: datetime | None = None
    created_at: datetime = Field(default_factory=utcnow)


class Replay(SQLModel, table=True):
    id: str = Field(primary_key=True)  # = sha256(blob)
    account_id: int = Field(index=True, foreign_key="account.id")
    playlist: str
    status: str = "queued"  # queued|parsing|scoring|done|failed
    parser_version: str | None = None
    error: str | None = None
    created_at: datetime = Field(default_factory=utcnow)


class Report(SQLModel, table=True):
    id: int | None = Field(default=None, primary_key=True)
    replay_id: str = Field(index=True, foreign_key="replay.id")
    player_id: str
    composite: float
    first_man: float
    second_man: float
    general: float
    licence: str
    player_type: str
    main_leak: str  # metric key
    focus_chapter: str  # deep-link slug/anchor
    confidence: str = "ok"  # ok|low_confidence
    score_config_version: str
    parser_version: str
    metrics_json: str  # full per-metric breakdown (raw + normalized)
    created_at: datetime = Field(default_factory=utcnow)


class LeaderboardEntry(SQLModel, table=True):
    """Materialized best locked-profile score per account per season (§5).

    Populated by a deferred follow-up; defined here so the schema is complete.
    """

    id: int | None = Field(default=None, primary_key=True)
    account_id: int = Field(index=True, foreign_key="account.id")
    season: str = Field(index=True)
    player_id: str
    report_id: int = Field(foreign_key="report.id")
    composite: float = Field(index=True)
    first_man: float
    second_man: float
    general: float
    uploaded_at: datetime

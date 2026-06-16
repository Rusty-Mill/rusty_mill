"""Shared fixtures: an in-memory DB, a fake scorer, and a TestClient.

The whole suite runs without the Rust toolchain or real `.replay` files — the
scoring capability is faked behind the [`Scorer`] protocol. A single shared
in-memory SQLite connection (StaticPool) backs both request sessions and the
background worker.
"""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient
from sqlalchemy.pool import StaticPool
from sqlmodel import Session, SQLModel, create_engine

from app import config, credits, db
from app.models import Account
from app.scoring import PlayerScore, ScoreResult


def sample_players() -> list[PlayerScore]:
    return [
        PlayerScore(
            player_id="Alice",
            composite=72.0,
            first_man=70.0,
            second_man=68.0,
            general=75.0,
            licence="Gold",
            player_type="Anchor",
            main_leak="overcommit_rate",
            focus_chapter="Controlled counterattacks",
            confidence="ok",
            metrics_json="[]",
        )
    ]


class FakeScorer:
    """A scoring adapter with no subprocess: returns canned reports, or fails."""

    def __init__(
        self,
        *,
        fail: bool = False,
        players: list[PlayerScore] | None = None,
        parser_version: str = "boxcars-fake",
        config_version: str = "score-test",
        report_html: str = "<html><body>fake report</body></html>",
    ) -> None:
        self.fail = fail
        self.players = sample_players() if players is None else players
        self.parser_version = parser_version
        self.config_version = config_version
        self.report_html = report_html
        self.calls = 0

    def score(self, blob: bytes, replay_id: str) -> ScoreResult:
        self.calls += 1
        if self.fail:
            raise RuntimeError("worker boom")
        return ScoreResult(
            self.parser_version,
            self.config_version,
            list(self.players),
            report_html=self.report_html,
        )


class FakePdfRenderer:
    """A renderer with no engine: returns a tiny valid-looking PDF, counts calls."""

    def __init__(self) -> None:
        self.calls = 0

    def render(self, html: str) -> bytes:
        self.calls += 1
        return b"%PDF-1.4\n" + html.encode()[:32]


@pytest.fixture
def engine(tmp_path, monkeypatch):
    monkeypatch.setattr(config.settings, "blob_dir", str(tmp_path / "blobs"))
    eng = create_engine(
        "sqlite://",
        connect_args={"check_same_thread": False},
        poolclass=StaticPool,
    )
    db.set_engine(eng)
    SQLModel.metadata.create_all(eng)
    yield eng
    db.set_engine(None)


@pytest.fixture
def session(engine):
    with Session(engine) as s:
        yield s


@pytest.fixture
def scorer():
    return FakeScorer()


@pytest.fixture
def failing_scorer():
    return FakeScorer(fail=True)


@pytest.fixture
def pdf_renderer():
    return FakePdfRenderer()


@pytest.fixture
def make_account(session):
    def _make(email: str = "u@example.com", *, owns_book: bool = True, credits_n: int = 5):
        account = Account(email=email, owns_book=owns_book)
        session.add(account)
        session.commit()
        session.refresh(account)
        if credits_n:
            credits.grant(session, account.id, credits_n, "monthly_grant")
            session.commit()
        return account

    return _make


def _client(scorer, pdf_renderer) -> TestClient:
    from app.main import app

    app.state.scorer = scorer
    app.state.pdf_renderer = pdf_renderer
    return TestClient(app)


@pytest.fixture
def client(engine, scorer, pdf_renderer):
    return _client(scorer, pdf_renderer)


@pytest.fixture
def failing_client(engine, failing_scorer, pdf_renderer):
    return _client(failing_scorer, pdf_renderer)

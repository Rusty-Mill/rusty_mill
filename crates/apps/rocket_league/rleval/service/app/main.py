"""FastAPI surface (spec §7).

Implemented here: upload (`POST /v1/replays`), status, report fetch, credit
balance, and lock-profile. The PDF endpoint, public leaderboard, purchase
webhook, and admin re-score are deferred follow-ups (see README).

Auth is a dev stub: the account is resolved from an ``X-Account-Email`` header
(get-or-create). Real authentication is the web layer's job.
"""

from __future__ import annotations

import json
from contextlib import asynccontextmanager

from fastapi import (
    BackgroundTasks,
    Depends,
    FastAPI,
    File,
    Form,
    Header,
    HTTPException,
    Request,
    UploadFile,
)
from pydantic import BaseModel
from sqlmodel import Session, select

from . import credits, leaderboard
from .config import settings
from .db import get_session, init_db
from .models import Account, LeaderboardEntry, Replay, Report
from .scoring import Scorer, SubprocessScorer
from .service import ingest, process, replay_id_for


@asynccontextmanager
async def lifespan(app: FastAPI):
    init_db()
    yield


app = FastAPI(title="Replay-Scoring Service", version="0.1.0", lifespan=lifespan)
# Default production scorer; tests override `app.state.scorer`.
app.state.scorer = SubprocessScorer(settings.worker_bin, settings.worker_timeout_s)


def get_scorer(request: Request) -> Scorer:
    return request.app.state.scorer


def current_account(
    session: Session = Depends(get_session),
    x_account_email: str = Header(..., alias="X-Account-Email"),
) -> Account:
    account = session.exec(
        select(Account).where(Account.email == x_account_email)
    ).first()
    if account is None:
        account = Account(email=x_account_email)
        session.add(account)
        session.commit()
        session.refresh(account)
    return account


def _report_dict(r: Report) -> dict:
    return {
        "id": r.id,
        "player_id": r.player_id,
        "composite": r.composite,
        "first_man": r.first_man,
        "second_man": r.second_man,
        "general": r.general,
        "licence": r.licence,
        "player_type": r.player_type,
        "main_leak": r.main_leak,
        "focus_chapter": r.focus_chapter,
        "confidence": r.confidence,
        "score_config_version": r.score_config_version,
        "parser_version": r.parser_version,
        "metrics": json.loads(r.metrics_json),
    }


@app.get("/healthz")
def healthz() -> dict:
    return {"ok": True}


@app.post("/v1/replays", status_code=202)
def upload_replay(
    background: BackgroundTasks,
    file: UploadFile = File(...),
    playlist: str = Form("unknown"),
    account: Account = Depends(current_account),
    session: Session = Depends(get_session),
    scorer: Scorer = Depends(get_scorer),
) -> dict:
    # Guards (§7): entitlement, then a non-empty/bounded blob, then credit.
    if not account.owns_book:
        raise HTTPException(403, "owns_book entitlement required")
    blob = file.file.read()
    if not blob:
        raise HTTPException(400, "empty upload")
    if len(blob) > settings.upload_max_bytes:
        raise HTTPException(413, "replay too large")

    rid = replay_id_for(blob)
    existing = session.get(Replay, rid)
    if existing is not None:  # idempotent re-upload (§9)
        return {"replay_id": rid, "status": existing.status, "deduped": True}

    if credits.balance(session, account.id) <= 0:
        raise HTTPException(402, "no credits")

    replay = ingest(session, account, blob, playlist)
    background.add_task(process, rid, scorer)
    return {"replay_id": rid, "status": replay.status, "deduped": False}


@app.get("/v1/replays/{replay_id}")
def replay_status(
    replay_id: str,
    account: Account = Depends(current_account),
    session: Session = Depends(get_session),
) -> dict:
    replay = session.get(Replay, replay_id)
    if replay is None or replay.account_id != account.id:
        raise HTTPException(404, "replay not found")
    return {
        "replay_id": replay.id,
        "status": replay.status,
        "playlist": replay.playlist,
        "parser_version": replay.parser_version,
        "error": replay.error,
    }


@app.get("/v1/reports/{replay_id}")
def get_reports(
    replay_id: str,
    account: Account = Depends(current_account),
    session: Session = Depends(get_session),
) -> dict:
    replay = session.get(Replay, replay_id)
    if replay is None or replay.account_id != account.id:
        raise HTTPException(404, "report not found")
    rows = session.exec(
        select(Report).where(Report.replay_id == replay_id)
    ).all()
    return {
        "replay_id": replay_id,
        "status": replay.status,
        "reports": [_report_dict(r) for r in rows],
    }


def _leaderboard_dict(rank: int, e: LeaderboardEntry) -> dict:
    return {
        "rank": rank,
        "player_id": e.player_id,  # the public locked-profile handle
        "composite": e.composite,
        "first_man": e.first_man,
        "second_man": e.second_man,
        "general": e.general,
        "uploaded_at": e.uploaded_at.isoformat(),
    }


@app.get("/v1/leaderboard")
def get_leaderboard(
    season: str | None = None,
    limit: int = 50,
    session: Session = Depends(get_session),
) -> dict:
    # Public feed; no account required. Defaults to the current season.
    season = season or leaderboard.current_season()
    limit = max(1, min(limit, 200))
    rows = leaderboard.top(session, season, limit)
    return {
        "season": season,
        "entries": [_leaderboard_dict(i + 1, e) for i, e in enumerate(rows)],
    }


@app.get("/v1/account/credits")
def account_credits(
    account: Account = Depends(current_account),
    session: Session = Depends(get_session),
) -> dict:
    return {
        "balance": credits.balance(session, account.id),
        "owns_book": account.owns_book,
        "locked_player_id": account.locked_player_id,
    }


class LockProfile(BaseModel):
    player_id: str


@app.post("/v1/account/lock-profile")
def lock_profile(
    body: LockProfile,
    account: Account = Depends(current_account),
    session: Session = Depends(get_session),
) -> dict:
    account.locked_player_id = body.player_id
    session.add(account)
    session.commit()
    return {"locked_player_id": account.locked_player_id}

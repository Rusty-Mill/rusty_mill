"""FastAPI surface (spec §7).

The full surface: upload, status, reports (+PDF), public leaderboard, credit
balance, lock-profile, the purchase webhook, and admin re-score. Scoring is
scheduled through a `JobQueue` port (in-process by default; see `queue.py`).

Auth is a dev stub: the account is resolved from an ``X-Account-Email`` header
(get-or-create). Real authentication is the web layer's job.
"""

from __future__ import annotations

import hashlib
import hmac
import json
import time
from contextlib import asynccontextmanager
from datetime import timedelta

from fastapi import (
    BackgroundTasks,
    Depends,
    FastAPI,
    File,
    Form,
    Header,
    HTTPException,
    Request,
    Response,
    UploadFile,
)
from pydantic import BaseModel, ValidationError
from sqlalchemy import func
from sqlmodel import Session, select

from . import blobs, credits, leaderboard, metrics, webhooks
from .config import settings
from .db import get_session, init_db
from .models import Account, LeaderboardEntry, Replay, Report, utcnow
from .pdf import PdfRenderer, PdfRenderError, WeasyPrintRenderer
from .queue import BackgroundTaskQueue, JobQueue
from .scoring import Scorer, SubprocessScorer
from .service import ingest, replay_id_for, rescore


@asynccontextmanager
async def lifespan(app: FastAPI):
    if settings.db_auto_create:  # dev; production runs Alembic migrations instead
        init_db()
    yield


class MetricsMiddleware:
    """Pure-ASGI RED metrics, recorded at response-start so background-task
    scoring (which runs *after* the response) isn't counted in request latency
    and isn't disturbed. Labels use the matched route template, not the raw path,
    to bound cardinality."""

    def __init__(self, app) -> None:
        self.app = app

    async def __call__(self, scope, receive, send) -> None:
        if scope["type"] != "http":
            await self.app(scope, receive, send)
            return
        start = time.perf_counter()

        async def send_wrapper(message) -> None:
            if message["type"] == "http.response.start":
                dt = time.perf_counter() - start
                route = scope.get("route")
                path = getattr(route, "path", None) or "unmatched"
                labels = {"method": scope["method"], "route": path}
                metrics.HTTP_REQUESTS.inc(status=str(message["status"]), **labels)
                metrics.HTTP_LATENCY.observe(dt, **labels)
            await send(message)

        await self.app(scope, receive, send_wrapper)


app = FastAPI(title="Replay-Scoring Service", version="0.1.0", lifespan=lifespan)
app.add_middleware(MetricsMiddleware)
# Default production adapters; tests override `app.state.{scorer,pdf_renderer}`.
app.state.scorer = SubprocessScorer(
    settings.worker_bin, settings.worker_timeout_s, settings.cache_canonical
)
app.state.pdf_renderer = WeasyPrintRenderer()


def _select_queue_factory():
    """Pick the JobQueue adapter from config (queue.py). Celery imports happen
    only when the broker backend is selected, so the in-process default never
    pulls in celery; tests can still override `app.state.queue_factory`."""
    if settings.queue_backend == "celery":
        from .queue import CeleryJobQueue

        queue = CeleryJobQueue()
        return lambda background, scorer: queue
    return None  # None → the in-process BackgroundTaskQueue default


app.state.queue_factory = _select_queue_factory()


def get_scorer(request: Request) -> Scorer:
    return request.app.state.scorer


def get_queue(
    request: Request,
    background: BackgroundTasks,
    scorer: Scorer = Depends(get_scorer),
) -> JobQueue:
    factory = request.app.state.queue_factory
    if factory is not None:
        return factory(background, scorer)
    return BackgroundTaskQueue(background, scorer)


def get_pdf_renderer(request: Request) -> PdfRenderer:
    return request.app.state.pdf_renderer


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


def _rate_limited(session: Session, account_id: int) -> bool:
    """True if the account has hit its new-upload rate limit (§7)."""
    cutoff = utcnow() - timedelta(seconds=settings.upload_rate_window_s)
    recent = session.exec(
        select(func.count(Replay.id)).where(
            Replay.account_id == account_id, Replay.created_at > cutoff
        )
    ).one()
    return recent >= settings.upload_rate_limit


@app.get("/healthz")
def healthz() -> dict:
    return {"ok": True}


@app.get("/metrics")
def metrics_endpoint() -> Response:
    return Response(
        metrics.render(), media_type="text/plain; version=0.0.4; charset=utf-8"
    )


@app.post("/v1/replays", status_code=202)
def upload_replay(
    file: UploadFile = File(...),
    playlist: str = Form("unknown"),
    account: Account = Depends(current_account),
    session: Session = Depends(get_session),
    queue: JobQueue = Depends(get_queue),
) -> dict:
    # Guards (§7): entitlement, then a non-empty/bounded blob, then credit.
    if not account.owns_book:
        metrics.UPLOADS.inc(outcome="forbidden")
        raise HTTPException(403, "owns_book entitlement required")
    blob = file.file.read()
    if not blob:
        metrics.UPLOADS.inc(outcome="empty")
        raise HTTPException(400, "empty upload")
    if len(blob) > settings.upload_max_bytes:
        metrics.UPLOADS.inc(outcome="too_large")
        raise HTTPException(413, "replay too large")

    rid = replay_id_for(blob)
    existing = session.get(Replay, rid)
    if existing is not None:  # idempotent re-upload (§9); doesn't count toward the limit
        metrics.UPLOADS.inc(outcome="deduped")
        return {"replay_id": rid, "status": existing.status, "deduped": True}

    if _rate_limited(session, account.id):
        metrics.UPLOADS.inc(outcome="rate_limited")
        raise HTTPException(429, "upload rate limit exceeded")
    if credits.balance(session, account.id) <= 0:
        metrics.UPLOADS.inc(outcome="no_credit")
        raise HTTPException(402, "no credits")

    replay = ingest(session, account, blob, playlist)
    queue.enqueue_scoring(rid)
    metrics.UPLOADS.inc(outcome="accepted")
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


@app.get("/v1/reports/{replay_id}/pdf")
def get_report_pdf(
    replay_id: str,
    account: Account = Depends(current_account),
    session: Session = Depends(get_session),
    renderer: PdfRenderer = Depends(get_pdf_renderer),
) -> Response:
    replay = session.get(Replay, replay_id)
    if replay is None or replay.account_id != account.id:
        raise HTTPException(404, "report not found")
    if replay.status != "done":
        raise HTTPException(409, f"report not ready (status={replay.status})")

    if not blobs.has_pdf(replay_id):  # render once, then cache (§4.10)
        if not blobs.has_html(replay_id):
            raise HTTPException(404, "no report html to render")
        try:
            blobs.store_pdf(replay_id, renderer.render(blobs.load_html(replay_id)))
        except PdfRenderError as e:
            raise HTTPException(503, str(e)) from e

    return Response(
        blobs.load_pdf(replay_id),
        media_type="application/pdf",
        headers={"Content-Disposition": f'inline; filename="{replay_id}.pdf"'},
    )


@app.post("/internal/rescore/{replay_id}")
def rescore_replay(
    replay_id: str,
    session: Session = Depends(get_session),
    scorer: Scorer = Depends(get_scorer),
) -> dict:
    # Admin/internal: re-run the (now-newer) scoring core, no re-upload (§9).
    replay = session.get(Replay, replay_id)
    if replay is None:
        raise HTTPException(404, "replay not found")
    if replay.status != "done":
        raise HTTPException(409, f"replay not scored (status={replay.status})")
    result = rescore(session, replay, scorer)
    return {
        "replay_id": replay_id,
        "reports": len(result.players),
        "score_config_version": result.score_config_version,
        "parser_version": result.parser_version,
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


class PurchaseEvent(BaseModel):
    email: str
    event_id: str  # provider event id (idempotency key)
    kind: str = webhooks.MONTHLY  # "monthly" | "topup"
    grant_book: bool = False


def _verify_webhook_signature(raw: bytes, signature: str | None) -> None:
    """Verify the provider's HMAC-SHA256 signature over the raw body (§8).

    A no-op when `RLS_WEBHOOK_SECRET` is unset (dev); otherwise `401` on a missing
    or mismatched `X-Signature`.
    """
    secret = settings.webhook_secret
    if not secret:
        return
    expected = hmac.new(secret.encode(), raw, hashlib.sha256).hexdigest()
    if not signature or not hmac.compare_digest(expected, signature):
        raise HTTPException(401, "invalid webhook signature")


@app.post("/internal/webhooks/purchase")
async def purchase_webhook(
    request: Request, session: Session = Depends(get_session)
) -> dict:
    # HMAC is over the exact raw bytes, so read the body before parsing.
    raw = await request.body()
    _verify_webhook_signature(raw, request.headers.get("X-Signature"))
    try:
        body = PurchaseEvent.model_validate_json(raw)
    except ValidationError as e:
        raise HTTPException(422, "invalid payload") from e
    account = webhooks.apply_purchase(
        session,
        email=body.email,
        event_id=body.event_id,
        kind=body.kind,
        grant_book=body.grant_book,
    )
    return {
        "account_id": account.id,
        "owns_book": account.owns_book,
        "balance": credits.balance(session, account.id),
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

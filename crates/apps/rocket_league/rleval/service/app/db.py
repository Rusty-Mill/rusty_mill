"""Database engine + session plumbing.

A module-global engine keeps wiring simple and lets tests swap in an in-memory
SQLite engine (`set_engine`). Both the request-scoped dependency (`get_session`)
and the background worker (`session_scope`) draw from the same engine, so a test
engine covers both paths.
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager

from sqlmodel import Session, SQLModel, create_engine

from .config import settings

_engine = None


def _connect_args(url: str) -> dict:
    # check_same_thread is SQLite-only (background tasks run in a worker thread);
    # passing it to other drivers (e.g. psycopg2) errors.
    return {"check_same_thread": False} if url.startswith("sqlite") else {}


def get_engine():
    global _engine
    if _engine is None:
        url = settings.database_url
        # pool_pre_ping keeps pooled Postgres connections healthy across drops.
        _engine = create_engine(
            url, connect_args=_connect_args(url), pool_pre_ping=True
        )
    return _engine


def set_engine(engine) -> None:
    """Override the process engine (tests). Pass ``None`` to reset."""
    global _engine
    _engine = engine


def init_db() -> None:
    SQLModel.metadata.create_all(get_engine())


def get_session() -> Iterator[Session]:
    """FastAPI dependency: a request-scoped session."""
    with Session(get_engine()) as session:
        yield session


@contextmanager
def session_scope() -> Iterator[Session]:
    """A standalone session for background work (not request-scoped)."""
    with Session(get_engine()) as session:
        yield session

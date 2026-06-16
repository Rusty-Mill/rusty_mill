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


def get_engine():
    global _engine
    if _engine is None:
        # check_same_thread=False: background tasks run in a worker thread.
        _engine = create_engine(
            settings.database_url, connect_args={"check_same_thread": False}
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

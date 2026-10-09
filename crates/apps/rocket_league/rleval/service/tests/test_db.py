"""Engine wiring: SQLite-only connect-args don't leak to other drivers (§5)."""

from __future__ import annotations

from app.db import _connect_args


def test_sqlite_gets_check_same_thread():
    assert _connect_args("sqlite:///./rls.db") == {"check_same_thread": False}


def test_non_sqlite_gets_no_sqlite_connect_args():
    # check_same_thread is SQLite-only; passing it to psycopg2 would error.
    assert _connect_args("postgresql+psycopg2://u@h/db") == {}

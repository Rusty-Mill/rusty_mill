"""Alembic migrations (§5): the baseline creates the schema and matches the models.

`alembic check` is the key guard — it fails if the SQLModel models and the
migration ever diverge, so a model change without a migration breaks CI.
"""

from __future__ import annotations

from pathlib import Path

import pytest

pytest.importorskip("alembic")

from alembic import command  # noqa: E402
from alembic.config import Config  # noqa: E402
from sqlalchemy import create_engine, inspect  # noqa: E402

from app import config  # noqa: E402

_SERVICE = Path(__file__).resolve().parents[1]
_TABLES = {"account", "creditledger", "replay", "report", "webhookevent", "leaderboardentry"}


def _alembic_cfg() -> Config:
    cfg = Config(str(_SERVICE / "alembic.ini"))
    cfg.set_main_option("script_location", str(_SERVICE / "migrations"))
    return cfg


def test_upgrade_head_creates_full_schema(tmp_path, monkeypatch):
    url = f"sqlite:///{tmp_path / 'm.db'}"
    monkeypatch.setattr(config.settings, "database_url", url)
    command.upgrade(_alembic_cfg(), "head")
    tables = set(inspect(create_engine(url)).get_table_names())
    assert _TABLES <= tables


def test_migrations_match_models_no_drift(tmp_path, monkeypatch):
    monkeypatch.setattr(config.settings, "database_url", f"sqlite:///{tmp_path / 'c.db'}")
    cfg = _alembic_cfg()
    command.upgrade(cfg, "head")
    command.check(cfg)  # raises if the models and the migration diverge

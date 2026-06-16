"""Runtime configuration, env-driven.

A plain mutable settings object (not pydantic-settings, to keep the dependency
surface small). Tests monkeypatch attributes directly; production reads env.
"""

from __future__ import annotations

import os

_MB = 1024 * 1024


class Settings:
    def __init__(self) -> None:
        # SQLAlchemy URL. SQLite for dev; point at Postgres in production (§5).
        self.database_url = os.getenv("RLS_DATABASE_URL", "sqlite:///./rls.db")
        # Where raw replay blobs are kept, content-addressed by replay_id (§9/§12).
        # NOTE: encryption-at-rest (§12) is a deferred follow-up.
        self.blob_dir = os.getenv("RLS_BLOB_DIR", "./blobs")
        # The Rust parse+score worker binary (`replay-scoring`) on PATH or absolute.
        self.worker_bin = os.getenv("RLS_WORKER_BIN", "replay-scoring")
        self.worker_timeout_s = int(os.getenv("RLS_WORKER_TIMEOUT_S", "300"))
        # The config version the service currently scores at; a report stored at an
        # older version is a re-score candidate (§9).
        self.score_config_version = os.getenv("RLS_SCORE_CONFIG_VERSION", "score-v1")
        # Monthly credit grant (§8); top-ups handled by the (deferred) webhook.
        self.monthly_grant = int(os.getenv("RLS_MONTHLY_GRANT", "20"))
        self.upload_max_bytes = int(os.getenv("RLS_UPLOAD_MAX_BYTES", str(25 * _MB)))


settings = Settings()

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
        # Auto-create tables on startup (dev). Set 0 in production, where schema is
        # managed by Alembic (`alembic upgrade head`) so create-all can't race it.
        self.db_auto_create = os.getenv("RLS_DB_AUTO_CREATE", "1") not in (
            "0",
            "",
            "false",
            "False",
        )
        # Artifact store (content-addressed by replay_id, §9/§12). "fs" keeps the
        # filesystem store at blob_dir; "s3" uses object storage (multi-node).
        self.blob_backend = os.getenv("RLS_BLOB_BACKEND", "fs")
        self.blob_dir = os.getenv("RLS_BLOB_DIR", "./blobs")
        self.s3_bucket = os.getenv("RLS_S3_BUCKET", "rls-blobs")
        self.s3_prefix = os.getenv("RLS_S3_PREFIX", "")
        self.s3_region = os.getenv("RLS_S3_REGION", "us-east-1")
        # Custom endpoint for S3-compatible stores (MinIO/localstack); "" = AWS.
        self.s3_endpoint_url = os.getenv("RLS_S3_ENDPOINT_URL", "") or None
        # The Rust parse+score worker binary (`replay-scoring`) on PATH or absolute.
        self.worker_bin = os.getenv("RLS_WORKER_BIN", "replay-scoring")
        self.worker_timeout_s = int(os.getenv("RLS_WORKER_TIMEOUT_S", "300"))
        # The config version the service currently scores at; a report stored at an
        # older version is a re-score candidate (§9).
        self.score_config_version = os.getenv("RLS_SCORE_CONFIG_VERSION", "score-v1")
        # Cache the (serde) canonical-match blob so a re-score skips re-parsing
        # (§9). Off by default: the canonical JSON is large (~3 MB gzipped vs a
        # ~1 MB replay) and parsing is cheap, so this trades storage for re-score
        # speed — enable it only where re-parse cost dominates.
        self.cache_canonical = os.getenv("RLS_CACHE_CANONICAL", "0") not in (
            "0",
            "",
            "false",
            "False",
        )
        # Monthly credit grant (§8); top-ups handled by the (deferred) webhook.
        self.monthly_grant = int(os.getenv("RLS_MONTHLY_GRANT", "20"))
        self.upload_max_bytes = int(os.getenv("RLS_UPLOAD_MAX_BYTES", str(25 * _MB)))
        # Per-account upload rate limit (§7): at most N new uploads per window.
        self.upload_rate_limit = int(os.getenv("RLS_UPLOAD_RATE_LIMIT", "30"))
        self.upload_rate_window_s = int(os.getenv("RLS_UPLOAD_RATE_WINDOW_S", "3600"))
        # Shared secret for verifying the purchase webhook's HMAC-SHA256 signature
        # (§8). Empty disables verification (dev only).
        self.webhook_secret = os.getenv("RLS_WEBHOOK_SECRET", "")
        # Master key for at-rest artifact encryption (§12). Empty = store verbatim.
        self.encryption_key = os.getenv("RLS_ENCRYPTION_KEY", "")
        # Job-queue backend (§4): "inprocess" (FastAPI BackgroundTasks) or "celery".
        self.queue_backend = os.getenv("RLS_QUEUE_BACKEND", "inprocess")
        self.celery_broker_url = os.getenv(
            "RLS_CELERY_BROKER_URL", "redis://localhost:6379/0"
        )


settings = Settings()

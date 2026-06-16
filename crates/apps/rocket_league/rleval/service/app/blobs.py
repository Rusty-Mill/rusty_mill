"""Content-addressed raw-replay storage.

Blobs are kept on disk keyed by ``replay_id`` (= sha256 of the bytes), so the
canonical worker can re-read them for a re-score without a re-upload (§9). A
filesystem store is the dev default; swap for object storage in production.
Encryption-at-rest (§12) is a deferred follow-up.
"""

from __future__ import annotations

from pathlib import Path

from .config import settings


def _dir() -> Path:
    p = Path(settings.blob_dir)
    p.mkdir(parents=True, exist_ok=True)
    return p


def store(replay_id: str, blob: bytes) -> None:
    (_dir() / replay_id).write_bytes(blob)


def load(replay_id: str) -> bytes:
    return (_dir() / replay_id).read_bytes()


def exists(replay_id: str) -> bool:
    return (_dir() / replay_id).exists()

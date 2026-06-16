"""Content-addressed artifact storage (raw replay + derived report HTML/PDF).

Everything is keyed by ``replay_id`` (= sha256 of the bytes): the raw blob (so the
worker can re-read it for a re-score without a re-upload, §9), the rendered report
HTML, and the cached report PDF (§4.10). A filesystem store is the dev default;
swap for object storage in production. Encryption-at-rest (§12) is deferred.
"""

from __future__ import annotations

import gzip
from pathlib import Path

from .config import settings


def _dir() -> Path:
    p = Path(settings.blob_dir)
    p.mkdir(parents=True, exist_ok=True)
    return p


def _path(replay_id: str, suffix: str = "") -> Path:
    return _dir() / f"{replay_id}{suffix}"


# --- raw replay blob ---
def store(replay_id: str, blob: bytes) -> None:
    _path(replay_id).write_bytes(blob)


def load(replay_id: str) -> bytes:
    return _path(replay_id).read_bytes()


def exists(replay_id: str) -> bool:
    return _path(replay_id).exists()


# --- derived report HTML (from `replay-scoring --html`) ---
def store_html(replay_id: str, html: str) -> None:
    _path(replay_id, ".html").write_text(html, encoding="utf-8")


def load_html(replay_id: str) -> str:
    return _path(replay_id, ".html").read_text(encoding="utf-8")


def has_html(replay_id: str) -> bool:
    return _path(replay_id, ".html").exists()


# --- cached report PDF (rendered once, §4.10) ---
def store_pdf(replay_id: str, pdf: bytes) -> None:
    _path(replay_id, ".pdf").write_bytes(pdf)


def load_pdf(replay_id: str) -> bytes:
    return _path(replay_id, ".pdf").read_bytes()


def has_pdf(replay_id: str) -> bool:
    return _path(replay_id, ".pdf").exists()


def delete_pdf(replay_id: str) -> None:
    """Drop the cached PDF (e.g. after a re-score changes the report)."""
    _path(replay_id, ".pdf").unlink(missing_ok=True)


# --- cached canonical-match blob (gzipped; for no-re-parse re-score, §9) ---
def store_canonical(replay_id: str, canonical_json: bytes) -> None:
    _path(replay_id, ".canonical.json.gz").write_bytes(gzip.compress(canonical_json))


def load_canonical(replay_id: str) -> bytes:
    """The raw canonical JSON (the worker's `--from-canonical` reads it raw)."""
    return gzip.decompress(_path(replay_id, ".canonical.json.gz").read_bytes())


def has_canonical(replay_id: str) -> bool:
    return _path(replay_id, ".canonical.json.gz").exists()

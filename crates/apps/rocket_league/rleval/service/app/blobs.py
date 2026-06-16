"""Content-addressed artifact storage (raw replay + derived report HTML/PDF).

Everything is keyed by ``replay_id`` (= sha256 of the *plaintext* bytes): the raw
blob (so the worker can re-read it for a re-score without a re-upload, §9), the
rendered report HTML, and the cached report PDF (§4.10). A filesystem store is the
dev default; swap for object storage in production.

Every artifact is written through the configured cipher (§12), so when
`RLS_ENCRYPTION_KEY` is set the bytes are encrypted-at-rest; the keys/ids stay the
plaintext hash, so dedupe is unaffected. With no key the cipher is a no-op and
on-disk bytes are byte-identical to before (no migration needed). The worker never
reads these files directly — it gets decrypted bytes via `load*` and writes its
own temp files — so encryption is transparent to scoring.
"""

from __future__ import annotations

import gzip
from pathlib import Path

from . import cipher
from .config import settings


def _dir() -> Path:
    p = Path(settings.blob_dir)
    p.mkdir(parents=True, exist_ok=True)
    return p


def _path(replay_id: str, suffix: str = "") -> Path:
    return _dir() / f"{replay_id}{suffix}"


# --- raw replay blob ---
def store(replay_id: str, blob: bytes) -> None:
    _path(replay_id).write_bytes(cipher.seal(blob))


def load(replay_id: str) -> bytes:
    return cipher.unseal(_path(replay_id).read_bytes())


def exists(replay_id: str) -> bool:
    return _path(replay_id).exists()


# --- derived report HTML (from `replay-scoring --html`) ---
def store_html(replay_id: str, html: str) -> None:
    _path(replay_id, ".html").write_bytes(cipher.seal(html.encode("utf-8")))


def load_html(replay_id: str) -> str:
    return cipher.unseal(_path(replay_id, ".html").read_bytes()).decode("utf-8")


def has_html(replay_id: str) -> bool:
    return _path(replay_id, ".html").exists()


# --- cached report PDF (rendered once, §4.10) ---
def store_pdf(replay_id: str, pdf: bytes) -> None:
    _path(replay_id, ".pdf").write_bytes(cipher.seal(pdf))


def load_pdf(replay_id: str) -> bytes:
    return cipher.unseal(_path(replay_id, ".pdf").read_bytes())


def has_pdf(replay_id: str) -> bool:
    return _path(replay_id, ".pdf").exists()


def delete_pdf(replay_id: str) -> None:
    """Drop the cached PDF (e.g. after a re-score changes the report)."""
    _path(replay_id, ".pdf").unlink(missing_ok=True)


# --- cached canonical-match blob (gzipped; for no-re-parse re-score, §9) ---
def store_canonical(replay_id: str, canonical_json: bytes) -> None:
    sealed = cipher.seal(gzip.compress(canonical_json))
    _path(replay_id, ".canonical.json.gz").write_bytes(sealed)


def load_canonical(replay_id: str) -> bytes:
    """The raw canonical JSON (the worker's `--from-canonical` reads it raw)."""
    return gzip.decompress(cipher.unseal(_path(replay_id, ".canonical.json.gz").read_bytes()))


def has_canonical(replay_id: str) -> bool:
    return _path(replay_id, ".canonical.json.gz").exists()

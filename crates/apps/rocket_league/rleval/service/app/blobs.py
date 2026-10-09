"""Content-addressed artifact storage (raw replay + derived report HTML/PDF).

Everything is keyed by ``replay_id`` (= sha256 of the *plaintext* bytes): the raw
blob (so the worker can re-read it for a re-score without a re-upload, §9), the
rendered report HTML, and the cached report PDF (§4.10).

Storage is a pluggable **`BlobStore`** port (§5): `FilesystemBlobStore` (the dev
default, at `blob_dir`) or `S3BlobStore` (object storage, so API and workers don't
share a local disk — true multi-node). The backend only moves opaque `(key,
bytes)`; the cipher (§12), gzip, and key/suffix composition live in this module,
so encryption-at-rest and dedupe work identically on either backend. The worker
never reads these directly — it gets decrypted bytes via `load*` and writes its
own temp files — so both the cipher and the backend are transparent to scoring.
"""

from __future__ import annotations

import gzip
from pathlib import Path
from typing import Protocol

from . import cipher
from .config import settings


class BlobStore(Protocol):
    def put(self, key: str, data: bytes) -> None: ...
    def get(self, key: str) -> bytes: ...
    def exists(self, key: str) -> bool: ...
    def delete(self, key: str) -> None: ...


class FilesystemBlobStore:
    """Files under a root directory (the dev default)."""

    def __init__(self, root: str) -> None:
        self._root = Path(root)

    def _path(self, key: str) -> Path:
        return self._root / key

    def put(self, key: str, data: bytes) -> None:
        self._root.mkdir(parents=True, exist_ok=True)
        self._path(key).write_bytes(data)

    def get(self, key: str) -> bytes:
        return self._path(key).read_bytes()

    def exists(self, key: str) -> bool:
        return self._path(key).exists()

    def delete(self, key: str) -> None:
        self._path(key).unlink(missing_ok=True)


class S3BlobStore:
    """Objects in an S3 (or S3-compatible: MinIO/localstack) bucket.

    boto3 is imported lazily so the filesystem default never needs it.
    """

    def __init__(
        self, bucket: str, prefix: str, region: str, endpoint_url: str | None
    ) -> None:
        import boto3

        self._bucket = bucket
        self._prefix = prefix
        self._s3 = boto3.client("s3", region_name=region, endpoint_url=endpoint_url)

    def _key(self, key: str) -> str:
        return f"{self._prefix}{key}"

    def put(self, key: str, data: bytes) -> None:
        self._s3.put_object(Bucket=self._bucket, Key=self._key(key), Body=data)

    def get(self, key: str) -> bytes:
        return self._s3.get_object(Bucket=self._bucket, Key=self._key(key))["Body"].read()

    def exists(self, key: str) -> bool:
        from botocore.exceptions import ClientError

        try:
            self._s3.head_object(Bucket=self._bucket, Key=self._key(key))
            return True
        except ClientError as e:
            if e.response["Error"]["Code"] in ("404", "NoSuchKey", "NotFound"):
                return False
            raise

    def delete(self, key: str) -> None:
        self._s3.delete_object(Bucket=self._bucket, Key=self._key(key))


_cache: tuple = (None, None)  # (config-key, BlobStore) — rebuilt when config changes


def _config_key() -> tuple:
    if settings.blob_backend == "s3":
        return ("s3", settings.s3_bucket, settings.s3_prefix, settings.s3_region,
                settings.s3_endpoint_url)
    return ("fs", settings.blob_dir)


def _build() -> BlobStore:
    if settings.blob_backend == "s3":
        return S3BlobStore(
            settings.s3_bucket, settings.s3_prefix, settings.s3_region,
            settings.s3_endpoint_url,
        )
    return FilesystemBlobStore(settings.blob_dir)


def _backend() -> BlobStore:
    global _cache
    key = _config_key()
    if _cache[0] != key:
        _cache = (key, _build())
    return _cache[1]


def reset_backend() -> None:
    """Drop the cached backend (tests; or after a config change)."""
    global _cache
    _cache = (None, None)


def _path(replay_id: str, suffix: str = "") -> Path:
    """Filesystem path for an artifact (fs backend only; used by tests)."""
    return Path(settings.blob_dir) / f"{replay_id}{suffix}"


# --- raw replay blob ---
def store(replay_id: str, blob: bytes) -> None:
    _backend().put(replay_id, cipher.seal(blob))


def load(replay_id: str) -> bytes:
    return cipher.unseal(_backend().get(replay_id))


def exists(replay_id: str) -> bool:
    return _backend().exists(replay_id)


# --- derived report HTML (from `replay-scoring --html`) ---
def store_html(replay_id: str, html: str) -> None:
    _backend().put(f"{replay_id}.html", cipher.seal(html.encode("utf-8")))


def load_html(replay_id: str) -> str:
    return cipher.unseal(_backend().get(f"{replay_id}.html")).decode("utf-8")


def has_html(replay_id: str) -> bool:
    return _backend().exists(f"{replay_id}.html")


# --- cached report PDF (rendered once, §4.10) ---
def store_pdf(replay_id: str, pdf: bytes) -> None:
    _backend().put(f"{replay_id}.pdf", cipher.seal(pdf))


def load_pdf(replay_id: str) -> bytes:
    return cipher.unseal(_backend().get(f"{replay_id}.pdf"))


def has_pdf(replay_id: str) -> bool:
    return _backend().exists(f"{replay_id}.pdf")


def delete_pdf(replay_id: str) -> None:
    """Drop the cached PDF (e.g. after a re-score changes the report)."""
    _backend().delete(f"{replay_id}.pdf")


# --- cached canonical-match blob (gzipped; for no-re-parse re-score, §9) ---
def store_canonical(replay_id: str, canonical_json: bytes) -> None:
    sealed = cipher.seal(gzip.compress(canonical_json))
    _backend().put(f"{replay_id}.canonical.json.gz", sealed)


def load_canonical(replay_id: str) -> bytes:
    """The raw canonical JSON (the worker's `--from-canonical` reads it raw)."""
    return gzip.decompress(cipher.unseal(_backend().get(f"{replay_id}.canonical.json.gz")))


def has_canonical(replay_id: str) -> bool:
    return _backend().exists(f"{replay_id}.canonical.json.gz")

"""At-rest encryption for stored artifacts (§12).

A small, pluggable cipher sits under the blob store: with no key configured the
default is `NullCipher` (a no-op — dev default), and with `RLS_ENCRYPTION_KEY`
set, `HmacAead` seals every artifact.

`HmacAead` is authenticated encryption composed from stdlib primitives only
(HMAC-SHA256): a CTR-mode keystream under **encrypt-then-MAC**, with the
encryption and MAC subkeys HKDF-separated from the master key. It stays the
dependency-free dev default. Production selects a vetted adapter behind the same
`seal`/`unseal` interface via ``RLS_CIPHER_BACKEND``:

* ``hmac`` (default) — `HmacAead`, stdlib only.
* ``aesgcm`` — `AesGcmCipher`, AES-256-GCM (needs the ``crypto`` extra:
  ``pip install 'rls-service[crypto]'``).
* ``kms`` — `KmsEnvelopeCipher`, a per-blob AES-256-GCM data key wrapped by AWS
  KMS (needs boto3 + the ``crypto`` extra).

Each adapter writes a distinct 4-byte MAGIC, so `unseal` dispatches by prefix and
a backend migration (e.g. hmac→aesgcm) keeps old blobs readable. Key *version*
(``RLS_CIPHER_KEY_VERSION``) is folded into key derivation: bumping it is a hard
rotation — old blobs then fail auth and must be re-encrypted.

Formats:
  HMAC   ``RLS1 ‖ nonce(16) ‖ tag(32) ‖ ciphertext``
  AESGCM ``AES1 ‖ nonce(12) ‖ ciphertext+tag``
  KMS    ``KMS1 ‖ wrapped_key_len(2) ‖ wrapped_key ‖ nonce(12) ‖ ciphertext+tag``
"""

from __future__ import annotations

import hashlib
import hmac
import os
from typing import Protocol

from .config import settings

_MAGIC = b"RLS1"
_NONCE = 16
_TAG = 32
_AES_MAGIC = b"AES1"
_KMS_MAGIC = b"KMS1"
_GCM_NONCE = 12  # NIST-recommended GCM nonce length


class Cipher(Protocol):
    def seal(self, plaintext: bytes) -> bytes: ...
    def unseal(self, blob: bytes) -> bytes: ...


class NullCipher:
    """No-op: artifacts are stored verbatim (no key configured)."""

    def seal(self, plaintext: bytes) -> bytes:
        return plaintext

    def unseal(self, blob: bytes) -> bytes:
        return blob


def _hkdf_expand(key: bytes, info: bytes, n: int = 32) -> bytes:
    """HKDF-Expand (RFC 5869) over HMAC-SHA256; one block covers n ≤ 32."""
    return hmac.new(key, info + b"\x01", hashlib.sha256).digest()[:n]


def _xor(a: bytes, b: bytes) -> bytes:
    n = len(a)
    return (int.from_bytes(a, "big") ^ int.from_bytes(b, "big")).to_bytes(n, "big")


class HmacAead:
    """Authenticated encryption from stdlib HMAC-SHA256 (see module docstring)."""

    def __init__(self, key: bytes, version: str = "0") -> None:
        master = hashlib.sha256(key).digest()  # normalize any key length to 32B
        # version "0" keeps the original info bytes, so blobs sealed before key
        # versioning still unseal; a non-zero version re-derives the subkeys.
        suffix = b"" if version == "0" else f"-{version}".encode()
        self._enc = _hkdf_expand(master, b"rls-enc" + suffix)
        self._mac = _hkdf_expand(master, b"rls-mac" + suffix)

    def _keystream(self, nonce: bytes, n: int) -> bytes:
        out = bytearray()
        counter = 0
        while len(out) < n:
            out += hmac.new(
                self._enc, nonce + counter.to_bytes(8, "big"), hashlib.sha256
            ).digest()
            counter += 1
        return bytes(out[:n])

    def seal(self, plaintext: bytes) -> bytes:
        nonce = os.urandom(_NONCE)
        ct = _xor(plaintext, self._keystream(nonce, len(plaintext)))
        tag = hmac.new(self._mac, _MAGIC + nonce + ct, hashlib.sha256).digest()
        return _MAGIC + nonce + tag + ct

    def unseal(self, blob: bytes) -> bytes:
        if blob[:4] != _MAGIC:
            raise ValueError("not an RLS-sealed blob")
        nonce = blob[4 : 4 + _NONCE]
        tag = blob[4 + _NONCE : 4 + _NONCE + _TAG]
        ct = blob[4 + _NONCE + _TAG :]
        expected = hmac.new(self._mac, _MAGIC + nonce + ct, hashlib.sha256).digest()
        if not hmac.compare_digest(expected, tag):
            raise ValueError("ciphertext authentication failed")
        return _xor(ct, self._keystream(nonce, len(ct)))


class AesGcmCipher:
    """AES-256-GCM with an HKDF-derived key (needs the ``crypto`` extra)."""

    def __init__(self, key: bytes, version: str = "0") -> None:
        try:
            from cryptography.hazmat.primitives.ciphers.aead import AESGCM
        except ImportError as e:  # pragma: no cover - exercised only without the dep
            raise RuntimeError(
                "the 'aesgcm' cipher backend needs the cryptography package "
                "(install the 'crypto' extra)"
            ) from e
        master = hashlib.sha256(key).digest()
        self._key = _hkdf_expand(master, f"rls-aes-{version}".encode(), 32)
        self._AESGCM = AESGCM

    def seal(self, plaintext: bytes) -> bytes:
        nonce = os.urandom(_GCM_NONCE)
        ct = self._AESGCM(self._key).encrypt(nonce, plaintext, _AES_MAGIC)
        return _AES_MAGIC + nonce + ct

    def unseal(self, blob: bytes) -> bytes:
        if blob[:4] != _AES_MAGIC:
            raise ValueError("not an AES-GCM-sealed blob")
        nonce = blob[4 : 4 + _GCM_NONCE]
        ct = blob[4 + _GCM_NONCE :]
        try:
            return self._AESGCM(self._key).decrypt(nonce, ct, _AES_MAGIC)
        except Exception as e:  # noqa: BLE001 - any AEAD failure is an auth failure
            raise ValueError("AES-GCM authentication failed") from e


class KmsEnvelopeCipher:
    """Per-blob AES-256-GCM data key wrapped by AWS KMS (envelope encryption).

    Each `seal` asks KMS for a fresh data key (so the master key never leaves KMS),
    encrypts with it, and stores the KMS-wrapped key in the blob header; `unseal`
    asks KMS to unwrap it. Needs boto3 (already a dep) + the ``crypto`` extra.
    """

    def __init__(self, kms_key_arn: str, region: str = "") -> None:
        try:
            import boto3
            from cryptography.hazmat.primitives.ciphers.aead import AESGCM
        except ImportError as e:  # pragma: no cover - exercised only without deps
            raise RuntimeError(
                "the 'kms' cipher backend needs boto3 and the cryptography package "
                "(install the 'crypto' extra)"
            ) from e
        self._kms = boto3.client("kms", region_name=region or None)
        self._arn = kms_key_arn
        self._AESGCM = AESGCM

    def seal(self, plaintext: bytes) -> bytes:
        resp = self._kms.generate_data_key(KeyId=self._arn, KeySpec="AES_256")
        data_key, wrapped = resp["Plaintext"], resp["CiphertextBlob"]
        nonce = os.urandom(_GCM_NONCE)
        ct = self._AESGCM(data_key).encrypt(nonce, plaintext, _KMS_MAGIC)
        del data_key  # Python can't truly wipe immutable bytes; drop the reference
        return _KMS_MAGIC + len(wrapped).to_bytes(2, "big") + wrapped + nonce + ct

    def unseal(self, blob: bytes) -> bytes:
        if blob[:4] != _KMS_MAGIC:
            raise ValueError("not a KMS-sealed blob")
        klen = int.from_bytes(blob[4:6], "big")
        wrapped = blob[6 : 6 + klen]
        nonce = blob[6 + klen : 6 + klen + _GCM_NONCE]
        ct = blob[6 + klen + _GCM_NONCE :]
        data_key = self._kms.decrypt(CiphertextBlob=wrapped)["Plaintext"]
        try:
            return self._AESGCM(data_key).decrypt(nonce, ct, _KMS_MAGIC)
        except Exception as e:  # noqa: BLE001 - any AEAD failure is an auth failure
            raise ValueError("KMS-envelope authentication failed") from e
        finally:
            del data_key


def _hmac() -> HmacAead:
    return HmacAead(settings.encryption_key.encode(), settings.cipher_key_version)


def _aesgcm() -> AesGcmCipher:
    return AesGcmCipher(settings.encryption_key.encode(), settings.cipher_key_version)


def _kms() -> KmsEnvelopeCipher:
    if not settings.kms_key_arn:
        raise ValueError("RLS_KMS_KEY_ARN is required for the 'kms' cipher backend")
    return KmsEnvelopeCipher(settings.kms_key_arn, settings.kms_region)


def active() -> Cipher:
    """Build the writer cipher for the configured backend (cheap; rebuilt per call,
    so a setting/key change takes effect immediately). With no key configured the
    hmac/aesgcm backends fall back to `NullCipher` (the dev default)."""
    backend = settings.cipher_backend.lower()
    if backend == "kms":
        return _kms()
    if backend in ("null", "") or not settings.encryption_key:
        return NullCipher()
    if backend == "hmac":
        return _hmac()
    if backend == "aesgcm":
        return _aesgcm()
    raise ValueError(f"unknown RLS_CIPHER_BACKEND {backend!r}")


def seal(plaintext: bytes) -> bytes:
    return active().seal(plaintext)


def unseal(blob: bytes) -> bytes:
    """Decrypt by the sealed blob's MAGIC, so blobs written by any backend stay
    readable across a backend migration. An unrecognized prefix is treated as
    stored-verbatim (the `NullCipher` path)."""
    magic = blob[:4]
    if magic == _MAGIC:
        return _hmac().unseal(blob)
    if magic == _AES_MAGIC:
        return _aesgcm().unseal(blob)
    if magic == _KMS_MAGIC:
        return _kms().unseal(blob)
    return blob

"""At-rest encryption for stored artifacts (§12).

A small, pluggable cipher sits under the blob store: with no key configured the
default is `NullCipher` (a no-op — dev default), and with `RLS_ENCRYPTION_KEY`
set, `HmacAead` seals every artifact.

`HmacAead` is authenticated encryption composed from stdlib primitives only
(HMAC-SHA256): a CTR-mode keystream under **encrypt-then-MAC**, with the
encryption and MAC subkeys HKDF-separated from the master key. The composition is
standard; the point is to keep the service dependency-free and testable
everywhere. A production deployment swaps a vetted AES-GCM/KMS adapter behind the
same `seal`/`unseal` interface (HMAC-CTR in pure Python is also slow on large
blobs — another reason to use AES-NI there).

Format: ``MAGIC(4) ‖ nonce(16) ‖ tag(32) ‖ ciphertext``.
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

    def __init__(self, key: bytes) -> None:
        master = hashlib.sha256(key).digest()  # normalize any key length to 32B
        self._enc = _hkdf_expand(master, b"rls-enc")
        self._mac = _hkdf_expand(master, b"rls-mac")

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


def active() -> Cipher:
    """Build the cipher for the configured key (cheap: two HMACs; not cached so a
    test or a key-rotation that changes the setting takes effect immediately)."""
    key = settings.encryption_key
    return HmacAead(key.encode()) if key else NullCipher()


def seal(plaintext: bytes) -> bytes:
    return active().seal(plaintext)


def unseal(blob: bytes) -> bytes:
    return active().unseal(blob)

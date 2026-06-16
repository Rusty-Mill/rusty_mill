"""At-rest cipher (§12): passthrough without a key, sealed AEAD with one."""

from __future__ import annotations

import pytest

from app import cipher, config


def _key(monkeypatch, key: str = "unit-test-master-key") -> None:
    monkeypatch.setattr(config.settings, "encryption_key", key)


def test_passthrough_without_key(monkeypatch):
    monkeypatch.setattr(config.settings, "encryption_key", "")
    assert cipher.seal(b"hello") == b"hello"
    assert cipher.unseal(b"hello") == b"hello"


def test_round_trips_with_key(monkeypatch):
    _key(monkeypatch)
    for pt in (b"", b"x", b"a longer secret payload \x00\xff" * 100):
        assert cipher.unseal(cipher.seal(pt)) == pt


def test_ciphertext_hides_plaintext(monkeypatch):
    _key(monkeypatch)
    secret = b"TOP-SECRET-DISTINCTIVE-MARKER-0123456789" * 4
    assert secret not in cipher.seal(secret)


def test_nonce_makes_each_seal_unique(monkeypatch):
    _key(monkeypatch)
    assert cipher.seal(b"same input") != cipher.seal(b"same input")


def test_tampering_is_detected(monkeypatch):
    _key(monkeypatch)
    sealed = bytearray(cipher.seal(b"important payload"))
    sealed[-1] ^= 0x01  # flip a ciphertext bit
    with pytest.raises(ValueError):
        cipher.unseal(bytes(sealed))


def test_wrong_key_is_rejected(monkeypatch):
    _key(monkeypatch, "key-a")
    sealed = cipher.seal(b"secret")
    _key(monkeypatch, "key-b")
    with pytest.raises(ValueError):
        cipher.unseal(sealed)

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


# --- key versioning + backward compatibility -------------------------------


def test_hmac_version_zero_is_byte_compatible_with_legacy():
    # A blob sealed by the original (no-version) HmacAead still unseals under the
    # default version "0" — key versioning didn't change the legacy derivation.
    legacy = cipher.HmacAead(b"unit-test-master-key").seal(b"payload")
    assert legacy[:4] == b"RLS1"
    assert cipher.HmacAead(b"unit-test-master-key", "0").unseal(legacy) == b"payload"


def test_key_version_bump_invalidates_old_blobs(monkeypatch):
    _key(monkeypatch)
    monkeypatch.setattr(config.settings, "cipher_backend", "hmac")
    monkeypatch.setattr(config.settings, "cipher_key_version", "0")
    sealed = cipher.seal(b"secret")
    monkeypatch.setattr(config.settings, "cipher_key_version", "1")
    with pytest.raises(ValueError):  # re-derived subkeys ⇒ auth fails (hard rotation)
        cipher.unseal(sealed)


# --- AES-256-GCM backend ----------------------------------------------------


def _aesgcm(monkeypatch):
    pytest.importorskip("cryptography")
    monkeypatch.setattr(config.settings, "cipher_backend", "aesgcm")
    _key(monkeypatch)


def test_aesgcm_round_trips(monkeypatch):
    _aesgcm(monkeypatch)
    for pt in (b"", b"x", b"a longer secret payload \x00\xff" * 100):
        assert cipher.unseal(cipher.seal(pt)) == pt


def test_aesgcm_hides_plaintext_and_is_unique(monkeypatch):
    _aesgcm(monkeypatch)
    secret = b"TOP-SECRET-DISTINCTIVE-MARKER" * 4
    sealed = cipher.seal(secret)
    assert sealed[:4] == b"AES1"
    assert secret not in sealed
    assert cipher.seal(secret) != cipher.seal(secret)


def test_aesgcm_tampering_is_detected(monkeypatch):
    _aesgcm(monkeypatch)
    sealed = bytearray(cipher.seal(b"important payload"))
    sealed[-1] ^= 0x01
    with pytest.raises(ValueError):
        cipher.unseal(bytes(sealed))


def test_unseal_dispatches_by_magic_across_backends(monkeypatch):
    # A backend migration (hmac → aesgcm) keeps old blobs readable: unseal picks
    # the cipher by the blob's MAGIC, not the currently configured backend.
    _aesgcm(monkeypatch)  # cryptography available + key set
    monkeypatch.setattr(config.settings, "cipher_backend", "hmac")
    old = cipher.seal(b"legacy blob")  # RLS1
    monkeypatch.setattr(config.settings, "cipher_backend", "aesgcm")
    new = cipher.seal(b"new blob")  # AES1
    assert old[:4] == b"RLS1" and new[:4] == b"AES1"
    assert cipher.unseal(old) == b"legacy blob"  # still readable under aesgcm
    assert cipher.unseal(new) == b"new blob"


# --- KMS-envelope backend (mocked with moto) --------------------------------


def test_kms_envelope_round_trips(monkeypatch):
    pytest.importorskip("cryptography")
    from moto import mock_aws  # noqa: PLC0415 - optional dep, imported in-test
    import boto3  # noqa: PLC0415

    with mock_aws():
        kms = boto3.client("kms", region_name="us-east-1")
        key_id = kms.create_key()["KeyMetadata"]["KeyId"]
        monkeypatch.setattr(config.settings, "cipher_backend", "kms")
        monkeypatch.setattr(config.settings, "kms_key_arn", key_id)
        monkeypatch.setattr(config.settings, "kms_region", "us-east-1")
        for pt in (b"", b"secret artifact \x00\xff" * 50):
            sealed = cipher.seal(pt)
            assert sealed[:4] == b"KMS1"
            assert cipher.unseal(sealed) == pt


def test_kms_requires_an_arn(monkeypatch):
    monkeypatch.setattr(config.settings, "cipher_backend", "kms")
    monkeypatch.setattr(config.settings, "kms_key_arn", "")
    with pytest.raises(ValueError, match="RLS_KMS_KEY_ARN"):
        cipher.seal(b"x")


def test_unknown_backend_raises(monkeypatch):
    _key(monkeypatch)
    monkeypatch.setattr(config.settings, "cipher_backend", "bogus")
    with pytest.raises(ValueError, match="unknown RLS_CIPHER_BACKEND"):
        cipher.seal(b"x")

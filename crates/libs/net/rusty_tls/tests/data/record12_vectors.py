#!/usr/bin/env python3
"""Generator for tests/handrolled_record12_kat.rs's vectors.

Builds TLS 1.2 AEAD records from the text of RFC 5246 section 6.2.3.3, RFC 5288
and RFC 7905 using Python `cryptography` (OpenSSL), independently of rusty_tls.
Run it and compare with the VECTORS table; it prints each vector as hex.

    python3 tests/data/record12_vectors.py
"""
import struct

from cryptography.hazmat.primitives.ciphers.aead import AESGCM, ChaCha20Poly1305


def pat(n, start):
    return bytes((start + i * 7) & 0xFF for i in range(n))


def aad(seq, typ, plaintext_len):
    # seq_num(8) || type(1) || version(2) || length(2); length is the PLAINTEXT's.
    return struct.pack(">Q", seq) + bytes([typ]) + b"\x03\x03" + struct.pack(">H", plaintext_len)


def gcm_record(key, salt, seq, typ, pt, explicit=None):
    explicit = struct.pack(">Q", seq) if explicit is None else explicit
    ct = AESGCM(key).encrypt(salt + explicit, pt, aad(seq, typ, len(pt)))
    body = explicit + ct
    return bytes([typ]) + b"\x03\x03" + struct.pack(">H", len(body)) + body


def chacha_record(key, iv, seq, typ, pt):
    nonce = bytearray(iv)
    for i, b in enumerate(struct.pack(">Q", seq)):
        nonce[4 + i] ^= b
    ct = ChaCha20Poly1305(key).encrypt(bytes(nonce), pt, aad(seq, typ, len(pt)))
    return bytes([typ]) + b"\x03\x03" + struct.pack(">H", len(ct)) + ct


if __name__ == "__main__":
    plans = (
        ("Aes128Gcm", 16, 4, gcm_record),
        ("Aes256Gcm", 32, 4, gcm_record),
        ("ChaCha20Poly1305", 32, 12, chacha_record),
    )
    for name, klen, flen, make in plans:
        key, fixed = pat(klen, 0x10), pat(flen, 0xA0)
        for seq, typ, pt in (
            (0, 22, b""),
            (1, 23, b"hello, tls 1.2"),
            (0x0102030405060708, 21, bytes([2, 40])),
            (255, 23, pat(100, 3)),
        ):
            print(name, seq, typ, make(key, fixed, seq, typ, pt).hex())

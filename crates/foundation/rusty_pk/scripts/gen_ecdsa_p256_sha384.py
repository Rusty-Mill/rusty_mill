#!/usr/bin/env python3
"""Generates P-256 / SHA-384 ECDSA vectors with an independent pure-Python
signer (Wycheproof has no such file and ring cannot sign with this pairing).
usage: gen_ecdsa_p256_sha384.py > tests/vectors/ecdsa_p256_sha384.txt
Each line: valid|invalid <pubkey hex> <message hex> <DER signature hex>
The hash is truncated to its leftmost 256 bits, per FIPS 186-4 section 6.4.
"""
import hashlib
import random

p = 0xFFFFFFFF00000001000000000000000000000000FFFFFFFFFFFFFFFFFFFFFFFF
n = 0xFFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551
G = (0x6B17D1F2E12C4247F8BCE6E563A440F277037D812DEB33A0F4A13945D898C296,
     0x4FE342E2FE1A7F9B8EE7EB4A7C0F9E162BCE33576B315ECECBB6406837BF51F5)


def add(P, Q):
    if P is None:
        return Q
    if Q is None:
        return P
    if P[0] == Q[0] and (P[1] + Q[1]) % p == 0:
        return None
    if P == Q:
        lam = (3 * P[0] * P[0] - 3) * pow(2 * P[1], -1, p) % p
    else:
        lam = (Q[1] - P[1]) * pow(Q[0] - P[0], -1, p) % p
    x = (lam * lam - P[0] - Q[0]) % p
    return (x, (lam * (P[0] - x) - P[1]) % p)


def mul(k, P):
    R = None
    while k:
        if k & 1:
            R = add(R, P)
        P = add(P, P)
        k >>= 1
    return R


def der_int(v):
    b = v.to_bytes((v.bit_length() + 7) // 8 or 1, "big")
    if b[0] & 0x80:
        b = b"\x00" + b
    return b"\x02" + bytes([len(b)]) + b


def sign(d, msg, rng):
    e = int.from_bytes(hashlib.sha384(msg).digest()[:32], "big") % n
    while True:
        k = rng.randrange(1, n)
        r = mul(k, G)[0] % n
        s = pow(k, -1, n) * (e + r * d) % n
        if r and s:
            body = der_int(r) + der_int(s)
            return b"\x30" + bytes([len(body)]) + body


rng = random.Random(0x384)
for i in range(24):
    d = rng.randrange(1, n)
    Q = mul(d, G)
    pub = b"\x04" + Q[0].to_bytes(32, "big") + Q[1].to_bytes(32, "big")
    msg = bytes(rng.randrange(256) for _ in range(rng.randrange(0, 200)))
    sig = sign(d, msg, rng)
    print("valid", pub.hex(), msg.hex(), sig.hex())
    bad = bytearray(msg or b"\x00")
    bad[0] ^= 1
    print("invalid", pub.hex(), bytes(bad).hex() if msg else "", sig.hex()) if msg else None
    # signature made over the SHA-256-truncation semantics would differ: a
    # signature over the wrong hash must not verify
    e256 = hashlib.sha256(msg).digest()
    wrong = bytearray(sig)
    wrong[-1] ^= 1
    print("invalid", pub.hex(), msg.hex(), bytes(wrong).hex())

#!/usr/bin/env python3
"""Generates RSA padding-forgery vectors: a throwaway 2048-bit key signs
deliberately malformed (or correct) encoded messages, so a verifier that
skips a padding check accepts something it must reject. Independent of
ring and of the Rust code.
usage: gen_rsa_forgeries.py > tests/vectors/rsa_forgeries.txt
Line: <name> <valid|invalid> <scheme> <RSAPublicKey DER hex> <message hex> <signature hex>
"""
import hashlib
import random

rng = random.Random(0x2048)


def is_prime(n, rounds=24):
    if n < 4:
        return n in (2, 3)
    if n % 2 == 0:
        return False
    d, s = n - 1, 0
    while d % 2 == 0:
        d //= 2
        s += 1
    for _ in range(rounds):
        a = rng.randrange(2, n - 1)
        x = pow(a, d, n)
        if x in (1, n - 1):
            continue
        for _ in range(s - 1):
            x = x * x % n
            if x == n - 1:
                break
        else:
            return False
    return True


def prime(bits):
    while True:
        c = rng.getrandbits(bits) | (3 << (bits - 2)) | 1
        if is_prime(c):
            return c


E = 65537
while True:
    p, q = prime(1024), prime(1024)
    n = p * q
    if n.bit_length() == 2048 and p != q and (p - 1) % E and (q - 1) % E:
        d = pow(E, -1, (p - 1) * (q - 1))
        break
K = 256


def der_len(n):
    return bytes([n]) if n < 128 else bytes([0x81, n]) if n < 256 else bytes([0x82, n >> 8, n & 255])


def der_int(v):
    b = v.to_bytes((v.bit_length() + 7) // 8, "big")
    b = b"\x00" + b if b[0] & 0x80 else b
    return b"\x02" + der_len(len(b)) + b


body = der_int(n) + der_int(E)
KEY = (b"\x30" + der_len(len(body)) + body).hex()
PREFIX256 = bytes.fromhex("3031300d060960864801650304020105000420")
PREFIX384 = bytes.fromhex("3041300d060960864801650304020205000430")


def sign_em(em):
    assert len(em) == K and int.from_bytes(em, "big") < n
    return pow(int.from_bytes(em, "big"), d, n).to_bytes(K, "big").hex()


def pkcs1(prefix, h, block=1, pad=0xFF, sep=0, extra=b""):
    pad_len = K - 3 - len(prefix) - len(h) - len(extra)
    return bytes([0, block]) + bytes([pad]) * pad_len + bytes([sep]) + prefix + h + extra


def emit(name, verdict, scheme, msg, em):
    print(name, verdict, scheme, KEY, msg.hex(), sign_em(em))


msg = b"forgery test message"
h = hashlib.sha256(msg).digest()
emit("pkcs1_valid", "valid", "pkcs1-sha256", msg, pkcs1(PREFIX256, h))
emit("pkcs1_block_type_2", "invalid", "pkcs1-sha256", msg, pkcs1(PREFIX256, h, block=2))
emit("pkcs1_block_type_0", "invalid", "pkcs1-sha256", msg, pkcs1(PREFIX256, h, block=0))
emit("pkcs1_pad_byte_fe", "invalid", "pkcs1-sha256", msg, pkcs1(PREFIX256, h, pad=0xFE))
emit("pkcs1_separator_nonzero", "invalid", "pkcs1-sha256", msg, pkcs1(PREFIX256, h, sep=1))
emit("pkcs1_wrong_hash", "invalid", "pkcs1-sha256", msg, pkcs1(PREFIX256, hashlib.sha256(b"x").digest()))
emit("pkcs1_sha384_prefix_for_sha256", "invalid", "pkcs1-sha256", msg, pkcs1(PREFIX384, h + b"\0" * 16))
emit("pkcs1_trailing_garbage", "invalid", "pkcs1-sha256", msg, pkcs1(PREFIX256, h, extra=b"\x00\x01"))
# a middle pad byte damaged, so only a full scan of the padding catches it
em = bytearray(pkcs1(PREFIX256, h))
em[100] = 0
emit("pkcs1_pad_hole", "invalid", "pkcs1-sha256", msg, bytes(em))

# --- PSS (sha256, salt 32) ---
EM_BITS = 2047
EM_LEN = (EM_BITS + 7) // 8  # 256
MASK = 0xFF >> (8 * EM_LEN - EM_BITS)


def mgf1(seed, length):
    out = b""
    c = 0
    while len(out) < length:
        out += hashlib.sha256(seed + c.to_bytes(4, "big")).digest()
        c += 1
    return out[:length]


def pss(m, salt, trailer=0xBC, ps_fill=0, one=1, set_top=False, mgf_seed_flip=False):
    mh = hashlib.sha256(m).digest()
    hh = hashlib.sha256(b"\0" * 8 + mh + salt).digest()
    ps_len = EM_LEN - len(salt) - 32 - 2
    db = bytes([ps_fill]) * ps_len + bytes([one]) + salt
    mask = mgf1(hh if not mgf_seed_flip else hh[:-1] + bytes([hh[-1] ^ 1]), len(db))
    masked = bytearray(x ^ y for x, y in zip(db, mask))
    masked[0] &= MASK
    if set_top:
        masked[0] = 0x80 | (masked[0] & 0x0F)  # stays below n (n >= 0x90...)
    return bytes(masked) + hh + bytes([trailer])


salt = bytes(rng.randrange(256) for _ in range(32))
emit("pss_valid", "valid", "pss-sha256", msg, pss(msg, salt))
emit("pss_bad_trailer", "invalid", "pss-sha256", msg, pss(msg, salt, trailer=0xBB))
emit("pss_ps_nonzero", "invalid", "pss-sha256", msg, pss(msg, salt, ps_fill=1))
emit("pss_missing_one", "invalid", "pss-sha256", msg, pss(msg, salt, one=0))
emit("pss_top_bit_set", "invalid", "pss-sha256", msg, pss(msg, salt, set_top=True))
emit("pss_wrong_mgf_seed", "invalid", "pss-sha256", msg, pss(msg, salt, mgf_seed_flip=True))
emit("pss_salt_20", "invalid", "pss-sha256", msg, pss(msg, salt[:20]))
emit("pss_salt_0", "invalid", "pss-sha256", msg, pss(msg, b""))
emit("pss_wrong_message", "invalid", "pss-sha256", b"other", pss(msg, salt))

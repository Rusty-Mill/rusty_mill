#!/usr/bin/env python3
"""Independent draft-irtf-cfrg-det-sigs-with-noise-05 section 4 fixtures.

Run from rusty_pk: python3 scripts/gen_hedged_ecdsa.py > tests/vectors/hedged_ecdsa.txt
Uses Python stdlib HMAC/hashlib (not rusty_sha2), integer modular arithmetic
and affine double-and-add (not rusty_pk's Montgomery/projective code).
This is a test reference, variable time and unsuitable for secret keys.

Padding is computed by rounding concatenated prefixes up to the hash block
size, independently of the Rust implementation's fixed padding constants.
Columns: curve message-length noise-byte K-after-f V-after-g first-k r s.
Private key is 0x11 repeated; message is 0x61 repeated; noise is noise-byte
repeated. Covers SHA message-padding boundaries and both noise/key block
boundaries. K/V/k pin the construction independently of signature verification.
"""
import hashlib
import hmac

CURVES = [
    (256, 'sha256',
     'ffffffff00000001000000000000000000000000ffffffffffffffffffffffff',
     'ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551',
     '6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296',
     '4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5',
     [0, 55, 56, 63, 64]),
    (384, 'sha384',
     'fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffeffffffff0000000000000000ffffffff',
     'ffffffffffffffffffffffffffffffffffffffffffffffffc7634d81f4372ddf581a0db248b0a77aecec196accc52973',
     'aa87ca22be8b05378eb1c71ef320ad746e1d3b628ba79b9859f741e082542a385502f25dbf55296c3a545e3872760ab7',
     '3617de4a96262c6f5d9e98bf9292dc29f8f41dbd289a147ce9da3113b5f0b8c00a60b1ce1d7e819d7a431d7c90ea0e5f',
     [0, 111, 112, 127, 128]),
]


def add(a, b, p):
    if a is None:
        return b
    if b is None:
        return a
    x, y = a
    u, v = b
    if x == u and (y + v) % p == 0:
        return None
    slope = ((3*x*x - 3) * pow(2*y, -1, p) if a == b
             else (v-y) * pow(u-x, -1, p)) % p
    w = (slope*slope - x-u) % p
    return w, (slope*(x-w)-y) % p


def mul(k, point, p):
    result = None
    for bit in bin(k)[2:]:
        result = add(result, result, p)
        if bit == '1':
            result = add(result, point, p)
    return result


for bits, name, p, n, gx, gy, lengths in CURVES:
    p, n, gx, gy = [int(v, 16) for v in (p, n, gx, gy)]
    size = bits // 8
    hash_fn = getattr(hashlib, name)
    block = hash_fn().block_size
    x = bytes([0x11]) * size
    d = int.from_bytes(x, 'big')
    pad = lambda value: value + bytes(-len(value) % block)
    for length in lengths:
        for noise_byte in [0, 0xa5]:
            z = bytes([noise_byte]) * size
            e = int.from_bytes(hash_fn(b'a' * length).digest(), 'big') % n
            h = e.to_bytes(size, 'big')
            key, v = bytes(size), bytes([1]) * size
            mac = lambda key, msg: hmac.digest(key, msg, name)
            for tag in [0, 1]:
                prefix = pad(v + bytes([tag]) + z)
                secret = pad(x)
                assert len(prefix) == ((2 * size + 1 + block - 1) // block) * block
                assert len(secret) == block
                key = mac(key, prefix + secret + h)
                v = mac(key, v)
            initial_v = v
            while True:
                v = mac(key, v)
                k = int.from_bytes(v, 'big')
                if 0 < k < n:
                    break
                key = mac(key, v + b'\x00')
                v = mac(key, v)
            r = mul(k, (gx, gy), p)[0] % n
            s = pow(k, -1, n) * (e + r*d) % n
            assert r and s
            print(bits, length, noise_byte, key.hex(), initial_v.hex(),
                  k.to_bytes(size, 'big').hex(), r.to_bytes(size, 'big').hex(),
                  s.to_bytes(size, 'big').hex())

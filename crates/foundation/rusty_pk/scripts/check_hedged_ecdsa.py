#!/usr/bin/env python3
"""Second, separately written check of the draft-05 hedged ECDSA fixtures.

Run from rusty_pk: python3 -I scripts/check_hedged_ecdsa.py tests/vectors/hedged_ecdsa.txt

It shares no code with gen_hedged_ecdsa.py or the Rust implementation. Padding is
`-(prefix length) mod block`. The curve arithmetic is affine and uses the curve
equation's `a = -3` directly. It first reproduces RFC 6979 A.2.5 and A.2.6 through
the plain path (Z = None), then recomputes r and s for every fixture line and exits
non-zero on any mismatch. Variable time; a test reference only, never for secret keys.
"""
import hashlib
import hmac
import sys

CURVES = {
    "P256": dict(
        p=0xffffffff00000001000000000000000000000000ffffffffffffffffffffffff,
        n=0xffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551,
        a=-3,
        gx=0x6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296,
        gy=0x4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5,
        h=hashlib.sha256, rlen=32, block=64),
    "P384": dict(
        p=0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffeffffffff0000000000000000ffffffff,
        n=0xffffffffffffffffffffffffffffffffffffffffffffffffc7634d81f4372ddf581a0db248b0a77aecec196accc52973,
        a=-3,
        gx=0xaa87ca22be8b05378eb1c71ef320ad746e1d3b628ba79b9859f741e082542a385502f25dbf55296c3a545e3872760ab7,
        gy=0x3617de4a96262c6f5d9e98bf9292dc29f8f41dbd289a147ce9da3113b5f0b8c00a60b1ce1d7e819d7a431d7c90ea0e5f,
        h=hashlib.sha384, rlen=48, block=128),
}

def add(c, P, Q):
    p = c["p"]
    if P is None: return Q
    if Q is None: return P
    (x1, y1), (x2, y2) = P, Q
    if x1 == x2 and (y1 + y2) % p == 0: return None
    if P == Q: l = (3 * x1 * x1 + c["a"]) * pow(2 * y1, -1, p) % p
    else: l = (y2 - y1) * pow(x2 - x1, -1, p) % p
    x3 = (l * l - x1 - x2) % p
    return (x3, (l * (x1 - x3) - y1) % p)

def mul(c, k, P):
    R = None
    for bit in bin(k)[2:]:
        R = add(c, R, R)
        if bit == "1": R = add(c, R, P)
    return R

def nonce(c, x, h1, Z):
    H, rlen, block = c["h"], c["rlen"], c["block"]
    hlen = H().digest_size
    xb, h1b = x.to_bytes(rlen, "big"), h1.to_bytes(rlen, "big")
    V, K = b"\x01" * hlen, b"\x00" * hlen
    def seed(tag):
        if Z is None:
            return V + tag + xb + h1b
        pad1 = (-(len(V) + 1 + len(Z))) % block
        pad2 = (-len(xb)) % block
        return V + tag + Z + b"\x00" * pad1 + xb + b"\x00" * pad2 + h1b
    K = hmac.new(K, seed(b"\x00"), H).digest(); V = hmac.new(K, V, H).digest()
    K = hmac.new(K, seed(b"\x01"), H).digest(); V = hmac.new(K, V, H).digest()
    while True:
        V = hmac.new(K, V, H).digest()
        k = int.from_bytes(V, "big")
        if 1 <= k < c["n"]: return k
        K = hmac.new(K, V + b"\x00", H).digest(); V = hmac.new(K, V, H).digest()

def sign(name, x, msg, Z):
    c = CURVES[name]; n = c["n"]
    z = int.from_bytes(c["h"](msg).digest(), "big") % n
    k = nonce(c, x, z, Z)
    r = mul(c, k, (c["gx"], c["gy"]))[0] % n
    s = pow(k, -1, n) * (z + r * x) % n
    return r, s


def main(path):
    x256 = 0xC9AFA9D845BA75166B5C215767B1D6934E50C3DB36E89B127B8A622B120F6721
    x384 = 0x6B9D3DAD2E1B8C1C05B19875B6659F4DE23C3B667BF297BA9AA47740787137D896D5724E4C70A825F872C9EA60D2EDF5
    assert sign("P256", x256, b"sample", None) == (
        0xEFD48B2AACB6A8FD1140DD9CD45E81D69D2C877B56AAF991C34D0EA84EAF3716,
        0xF7CB1C942D657C41D436C7A1B6E29F65F3E900DBB9AFF4064DC4AB2F843ACDA8), "RFC 6979 A.2.5"
    assert sign("P384", x384, b"sample", None) == (
        0x94EDBB92A5ECB8AAD4736E56C691916B3F88140666CE9FA73D64C4EA95AD133C81A648152E44ACF96E36DD1E80FABE46,
        0x99EF4AEB15F178CEA1FE40DB2603138F130E740A19624526203B6351D0A3A94FA329C145786E679E7B82C71A38628AC8), "RFC 6979 A.2.6"
    count = 0
    with open(path) as lines:
        for line in lines:
            f = line.split()
            name, rlen = ("P256", 32) if f[0] == "256" else ("P384", 48)
            x = int.from_bytes(b"\x11" * rlen, "big")
            got = sign(name, x, b"a" * int(f[1]), bytes([int(f[2])]) * rlen)
            if got != (int(f[6], 16), int(f[7], 16)):
                sys.exit(f"mismatch: {line.strip()}")
            count += 1
    print(f"RFC 6979 A.2.5/A.2.6 reproduced; {count} fixtures agree")


if __name__ == "__main__":
    main(sys.argv[1])

#!/usr/bin/env python3
"""Regenerate oidc_fixture.json: a throwaway RSA key's JWKS plus RS256 ID tokens.

Used by the unit tests in app/src/oidc.rs. The private key exists only inside
this run and is never written out, so the committed fixture holds nothing secret.
Tokens use fixed claims (audience client-123, nonce test-nonce) and exp in 2100,
so they stay valid; the "expired" one expired in 2001.

    python3 app/tests/fixtures/gen_oidc_fixture.py > app/tests/fixtures/oidc_fixture.json

Needs only Python 3.10+ and the `openssl` command line tool.
"""
import base64
import json
import subprocess
import tempfile
from pathlib import Path

FAR_FUTURE = 4102444800  # 2100-01-01
LONG_AGO = 978307200  # 2001-01-01
ISS = "https://accounts.google.com"


def b64u(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def openssl(*args: str, stdin: bytes | None = None) -> bytes:
    return subprocess.run(["openssl", *args], input=stdin, check=True,
                          capture_output=True).stdout


# The key lives in a temp dir that is deleted on exit; it is never printed.
_tmp = tempfile.TemporaryDirectory()
KEY = str(Path(_tmp.name) / "key.pem")
openssl("genrsa", "-out", KEY, "2048")

modulus_hex = openssl("rsa", "-in", KEY, "-noout", "-modulus").decode().strip().split("=")[1]
modulus = bytes.fromhex(modulus_hex)
jwks = {"keys": [{"kty": "RSA", "kid": "kid-1", "use": "sig", "alg": "RS256",
                  "n": b64u(modulus), "e": b64u((65537).to_bytes(3, "big"))}]}


def sign(claims: dict, kid="kid-1", alg="RS256") -> str:
    header = b64u(json.dumps({"alg": alg, "typ": "JWT", "kid": kid}).encode())
    payload = b64u(json.dumps(claims).encode())
    if alg == "none":
        return f"{header}.{payload}."
    sig = openssl("dgst", "-sha256", "-sign", KEY, stdin=f"{header}.{payload}".encode())
    return f"{header}.{payload}.{b64u(sig)}"


def claims(**over):
    base = {"iss": ISS, "aud": "client-123", "sub": "1234567890",
            "email": "alice@example.com", "email_verified": True,
            "nonce": "test-nonce", "iat": LONG_AGO, "exp": FAR_FUTURE}
    base.update(over)
    return base


valid = sign(claims())
head, payload, sig = valid.split(".")
forged = claims(email="bob@example.com")  # bob is allowlisted; alice's signature is reused
tampered = f"{head}.{b64u(json.dumps(forged).encode())}.{sig}"

tokens = {
    "valid": valid,
    "wrong_audience": sign(claims(aud="someone-else")),
    "expired": sign(claims(exp=LONG_AGO + 3600)),
    "wrong_nonce": sign(claims(nonce="other-nonce")),
    "wrong_issuer": sign(claims(iss="https://evil.example")),
    "tampered": tampered,
    "alg_none": sign(claims(), alg="none"),
    "unknown_kid": sign(claims(), kid="rotated-away"),
    "email_unverified": sign(claims(email_verified=False)),
    "unlisted_email": sign(claims(email="mallory@example.com")),
}
print(json.dumps({"jwks": jwks, "tokens": tokens}, indent=1))

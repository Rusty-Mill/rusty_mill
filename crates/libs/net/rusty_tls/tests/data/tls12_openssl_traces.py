#!/usr/bin/env python3
"""Generator for tests/data/tls12_openssl_traces.rs.

Runs real OpenSSL TLS 1.2 handshakes (s_server and s_client) through a recording
proxy and writes down everything a TLS 1.2 key derivation can be checked
against, as a Rust source file the tests `include!`.

Per capture:
  * the hello randoms, the master secret OpenSSL logged (NSS key log), and, for
    RSA key exchange only, the pre-master secret, recovered by decrypting
    ClientKeyExchange with the throwaway server key;
  * the handshake messages in order, exactly as they crossed the wire;
  * the first encrypted record in each direction (the Finished) and the first
    application-data record in each direction.

A test that derives the keys from the first three and decrypts the last two with
them has checked the PRF, the extended master secret, the key block's seed order
and partition, `Finished`, and the TLS 1.2 record layer against an
implementation nobody here wrote.

The key is generated fresh into a temp directory and never stored. Re-running
changes every value; the committed file is one run.

    python3 tests/data/tls12_openssl_traces.py > tests/data/tls12_openssl_traces.rs
"""
import hashlib
import os
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric import padding

SUITES = [
    # (openssl cipher, Aead variant, PRF hash, rsa key exchange?)
    ("AES128-GCM-SHA256", "Aes128Gcm", "Sha256", True),
    ("AES256-GCM-SHA384", "Aes256Gcm", "Sha384", True),
    ("ECDHE-RSA-AES128-GCM-SHA256", "Aes128Gcm", "Sha256", False),
    ("ECDHE-RSA-AES256-GCM-SHA384", "Aes256Gcm", "Sha384", False),
    ("ECDHE-RSA-CHACHA20-POLY1305", "ChaCha20Poly1305", "Sha256", False),
]
REQUEST = b"GET / HTTP/1.0\r\n\r\n"


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def relay(listen_sock, target_port, captured):
    """Accept one connection and relay it, recording both directions."""
    client, _ = listen_sock.accept()
    server = socket.create_connection(("127.0.0.1", target_port))
    captured["c2s"], captured["s2c"] = bytearray(), bytearray()

    def pump(src, dst, key):
        try:
            while True:
                data = src.recv(65536)
                if not data:
                    break
                captured[key].extend(data)
                dst.sendall(data)
        except OSError:
            pass
        finally:
            try:
                dst.shutdown(socket.SHUT_WR)
            except OSError:
                pass

    threads = [
        threading.Thread(target=pump, args=(client, server, "c2s")),
        threading.Thread(target=pump, args=(server, client, "s2c")),
    ]
    for t in threads:
        t.start()
    for t in threads:
        t.join(timeout=20)
    client.close()
    server.close()


def records(stream):
    out, i = [], 0
    while i + 5 <= len(stream):
        typ, ver, length = stream[i], stream[i + 1 : i + 3], struct.unpack(">H", stream[i + 3 : i + 5])[0]
        body = bytes(stream[i + 5 : i + 5 + length])
        assert len(body) == length, "truncated capture"
        out.append((typ, bytes(ver), body, bytes(stream[i : i + 5 + length])))
        i += 5 + length
    return out


def handshake_messages(recs):
    """Cleartext handshake messages: type-22 records before the first CCS."""
    blob = b""
    for typ, _ver, body, _raw in recs:
        if typ == 20:
            break
        if typ == 22:
            blob += body
    msgs, i = [], 0
    while i + 4 <= len(blob):
        n = int.from_bytes(blob[i + 1 : i + 4], "big")
        msgs.append(blob[i : i + 4 + n])
        i += 4 + n
    assert i == len(blob), "handshake messages split across a record boundary unexpectedly"
    return msgs


def after_ccs(recs, typ):
    seen = False
    for t, _v, _b, raw in recs:
        if t == 20:
            seen = True
        elif seen and t == typ:
            return raw
    raise SystemExit(f"no record of type {typ} after CCS")


def capture(cipher, tmp, key, cert):
    sport, pport = free_port(), None
    keylog = os.path.join(tmp, "keylog.txt")
    open(keylog, "w").close()
    srv = subprocess.Popen(
        ["openssl", "s_server", "-accept", str(sport), "-cert", cert, "-key", key, "-tls1_2",
         "-cipher", cipher + ":@SECLEVEL=0", "-no_ticket", "-www", "-quiet"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(0.6)
    lsock = socket.socket()
    lsock.bind(("127.0.0.1", 0))
    lsock.listen(1)
    pport = lsock.getsockname()[1]
    captured = {}
    t = threading.Thread(target=relay, args=(lsock, sport, captured))
    t.start()
    try:
        subprocess.run(
            ["openssl", "s_client", "-connect", f"127.0.0.1:{pport}", "-tls1_2",
             "-cipher", cipher + ":@SECLEVEL=0", "-no_ticket", "-keylogfile", keylog, "-quiet", "-ign_eof"],
            input=REQUEST, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=20)
    finally:
        t.join(timeout=25)
        srv.kill()
        lsock.close()
    return captured, keylog


def hexs(b):
    return b.hex()


def main():
    out = [
        "// @generated by tests/data/tls12_openssl_traces.py -- do not edit by hand.",
        "// Real OpenSSL TLS 1.2 handshakes; see the generator for what each field is.",
        "",
        "const TRACES: &[Trace] = &[",
    ]
    with tempfile.TemporaryDirectory() as tmp:
        key, cert = os.path.join(tmp, "key.pem"), os.path.join(tmp, "cert.pem")
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", key,
                        "-out", cert, "-subj", "/CN=trace.test", "-days", "2"],
                       check=True, capture_output=True)
        priv = serialization.load_pem_private_key(open(key, "rb").read(), None)

        for cipher, aead, hashname, rsa_kx in SUITES:
            captured, keylog = capture(cipher, tmp, key, cert)
            c2s, s2c = records(captured["c2s"]), records(captured["s2c"])
            cmsgs, smsgs = handshake_messages(c2s), handshake_messages(s2c)

            ch, cke = cmsgs[0], cmsgs[1]
            sh = smsgs[0]
            assert ch[0] == 1 and sh[0] == 2 and cke[0] == 16, (ch[0], sh[0], cke[0])
            client_random, server_random = ch[6:38], sh[6:38]

            # Extended master secret must have been negotiated: extension 0x0017 in both hellos.
            assert b"\x00\x17\x00\x00" in ch and b"\x00\x17\x00\x00" in sh, "OpenSSL did not use extended master secret"

            master = None
            for line in open(keylog):
                if not line.strip() or line.startswith("#"):
                    continue
                label, cr, ms = line.split()
                if label == "CLIENT_RANDOM" and bytes.fromhex(cr) == client_random:
                    master = bytes.fromhex(ms)
            assert master and len(master) == 48

            pre_master = None
            if rsa_kx:
                n = struct.unpack(">H", cke[4:6])[0]
                pre_master = priv.decrypt(cke[6 : 6 + n], padding.PKCS1v15())
                assert len(pre_master) == 48

            # Handshake messages in transcript order up to and including ClientKeyExchange.
            order = [cmsgs[0]] + smsgs + [cmsgs[1]]
            # server messages: ServerHello, Certificate, [ServerKeyExchange], ServerHelloDone
            assert smsgs[-1][0] == 14, "last cleartext server message should be ServerHelloDone"
            transcript = order

            c_fin = after_ccs(c2s, 22)
            s_fin = after_ccs(s2c, 22)
            c_app = after_ccs(c2s, 23)
            s_app = after_ccs(s2c, 23)

            out.append("    Trace {")
            out.append(f'        cipher: "{cipher}",')
            out.append(f"        aead: Aead::{aead},")
            out.append(f"        hash: Hash::{hashname},")
            out.append(f'        client_random: "{hexs(client_random)}",')
            out.append(f'        server_random: "{hexs(server_random)}",')
            out.append(f'        master_secret: "{hexs(master)}",')
            out.append(f'        pre_master_secret: {"Some(" + chr(34) + hexs(pre_master) + chr(34) + ")" if pre_master else "None"},')
            out.append("        transcript_to_client_key_exchange: &[")
            for m in transcript:
                out.append(f'            "{hexs(m)}",')
            out.append("        ],")
            out.append(f'        client_finished_record: "{hexs(c_fin)}",')
            out.append(f'        server_finished_record: "{hexs(s_fin)}",')
            out.append(f'        client_app_record: "{hexs(c_app)}",')
            out.append(f'        server_app_record: "{hexs(s_app)}",')
            out.append(f'        client_request: "{hexs(REQUEST)}",')
            out.append("    },")
            print(f"captured {cipher}", file=sys.stderr)
    out.append("];")
    print("\n".join(out))


if __name__ == "__main__":
    main()

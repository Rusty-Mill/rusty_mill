#!/usr/bin/env python3
"""Minimal RFC 5389 STUN server for the interop harness.

Headscale's embedded STUN doesn't answer in this environment and public STUN
servers aren't reachable (UDP egress is blocked), so the direct-path harness
runs this tiny responder. It replies to binding requests with the source
address in XOR-MAPPED-ADDRESS.

Usage: stun_server.py <bind-ip> [port]   (port defaults to 3478)
"""
import socket
import struct
import sys

COOKIE = b"\x21\x12\xa4\x42"


def xor_mapped_address(tx: bytes, ip: str, port: int) -> bytes:
    fam = 0x01  # IPv4
    xport = port ^ 0x2112
    key = COOKIE + tx
    addr = bytes(b ^ key[i] for i, b in enumerate(socket.inet_aton(ip)))
    return struct.pack(">BBH", 0, fam, xport) + addr


def response(tx: bytes, src) -> bytes:
    attr = xor_mapped_address(tx, src[0], src[1])
    hdr = b"\x01\x01" + struct.pack(">H", 4 + len(attr)) + COOKIE + tx
    return hdr + struct.pack(">HH", 0x0020, len(attr)) + attr


def main():
    bind_ip = sys.argv[1] if len(sys.argv) > 1 else "0.0.0.0"
    port = int(sys.argv[2]) if len(sys.argv) > 2 else 3478
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.bind((bind_ip, port))
    sys.stderr.write(f"STUN server on {bind_ip}:{port}\n")
    sys.stderr.flush()
    while True:
        data, src = s.recvfrom(512)
        if len(data) >= 20 and data[4:8] == COOKIE and data[0:2] == b"\x00\x01":
            s.sendto(response(data[8:20], src), src)


if __name__ == "__main__":
    main()

---
category: Added
changelog: rusty_tls native engine (gated): NativeTlsStream::tls13_only() offers TLS 1.3 alone and never runs the TLS 1.2 client; a TLS 1.2-only peer is refused. Off by default; nothing is wired.
---
## 2026-10-14 - rusty_tls: a TLS 1.3-only option on the native stream

- **Added (gated):** `NativeTlsStream::tls13_only()`. The ClientHello then carries only TLS 1.3 in `supported_versions`, no TLS 1.2 suite and no `TLS_FALLBACK_SCSV`, and the handshake runs the TLS 1.3 client alone, so a peer that will not speak 1.3 is refused with `protocol_version` instead of negotiated down. This is the enforced form of the first consumer's "TLS 1.3 only" scope (ADR-0002 Amendment 1, condition 4).
- **Unchanged:** the default still offers both versions, `TlsStream` and the rustls default are untouched, and no consumer is wired. The handshake loop is shared by both paths through a small private trait.
- **Fail closed:** `tls13_only()` returns `io::Result<Self>` and is `InvalidInput` once the handshake has started, so a late call cannot leave a caller believing an established TLS 1.2 connection is 1.3-only.
- **Fixed (review):** the TLS 1.3 client now bounds pre-ServerHello handshake reassembly at the same 128 KiB the dual-version client already applied. The 1.3-only path drives that client directly and would otherwise have held up to ~16 MiB of a declared-length message from an unauthenticated peer.
- **Tests:** the hello is parsed and checked (versions exactly `[0x0304]`, none of `CipherSuite12::SUPPORTED`, no SCSV, with the default hello as a control); a TLS 1.3 server completes and echoes; a TLS 1.2-only server is refused and the stream stays failed; a late call on an established TLS 1.2 stream is refused; oversized fragmented handshake input is refused (`InvalidData`, permanent) after at most nine of a hundred records are read. With the option made a no-op, the hello and refusal tests fail.

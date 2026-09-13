//! Shared cap for provider SSE/NDJSON response-stream accumulators.
//!
//! Round 7 (#defect) — `chat_stream_with` (and, for Ollama,
//! `chat_turn_with_tools`) in `openai.rs`, `anthropic.rs`, and
//! `ollama.rs` all reassemble newline-delimited records (`data:` SSE
//! lines or NDJSON lines) from a chunked `bytes_stream()` by extending a
//! `Vec<u8>` accumulator and draining it up to each `\n`. A malicious or
//! misbehaving upstream provider endpoint that never sends a newline —
//! or that streams gigabytes between newlines — would otherwise grow
//! that buffer without bound (OOM `DoS` reachable from provider-controlled
//! response bytes).
//!
//! Mirrors the read-side cap `nexus-linkpreview` applies at the
//! transport layer (`MAX_BODY_BYTES`, issue #78): bound the number of
//! bytes accepted before a terminator is found, and fail loudly instead
//! of growing forever.

use crate::error::AiError;

/// Maximum number of bytes a single SSE/NDJSON line accumulator may hold
/// before a `\n` terminator is found. Matches `nexus-linkpreview`'s
/// `MAX_BODY_BYTES` (512 KiB) — comfortably larger than any legitimate
/// single streamed record from these APIs, while bounding the worst
/// case an adversarial or buggy endpoint can inflict.
pub(crate) const MAX_STREAM_BUFFER_BYTES: usize = 512 * 1024;

/// Append `bytes` to `buf`, rejecting the append instead of growing
/// `buf` past [`MAX_STREAM_BUFFER_BYTES`] with no newline found yet.
///
/// Call this at every `bytes_stream()` chunk boundary in place of a bare
/// `buf.extend_from_slice(&bytes)` so a provider that never terminates a
/// line can't exhaust memory.
pub(crate) fn push_stream_bytes(buf: &mut Vec<u8>, bytes: &[u8]) -> Result<(), AiError> {
    if buf.len() + bytes.len() > MAX_STREAM_BUFFER_BYTES {
        return Err(AiError::Provider(format!(
            "provider stream line exceeded {MAX_STREAM_BUFFER_BYTES} bytes without a newline terminator"
        )));
    }
    buf.extend_from_slice(bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_bytes_within_cap() {
        let mut buf = Vec::new();
        push_stream_bytes(&mut buf, b"hello\n").unwrap();
        assert_eq!(buf, b"hello\n");
    }

    /// A single unterminated "line" (no `\n`) that keeps growing past
    /// the cap must be rejected rather than silently accepted — this is
    /// the exact OOM shape a malicious/misbehaving provider endpoint
    /// could otherwise trigger.
    #[test]
    fn rejects_oversized_unterminated_chunk() {
        let mut buf = Vec::new();
        let oversized = vec![b'x'; MAX_STREAM_BUFFER_BYTES + 1];
        let err = push_stream_bytes(&mut buf, &oversized).unwrap_err();
        assert!(matches!(err, AiError::Provider(_)));
        // Buffer must NOT have grown — the whole point of the cap.
        assert!(buf.is_empty());
    }

    #[test]
    fn rejects_when_accumulated_total_exceeds_cap_across_chunks() {
        let mut buf = vec![b'x'; MAX_STREAM_BUFFER_BYTES - 1];
        let err = push_stream_bytes(&mut buf, b"ab").unwrap_err();
        assert!(matches!(err, AiError::Provider(_)));
        assert_eq!(buf.len(), MAX_STREAM_BUFFER_BYTES - 1);
    }
}

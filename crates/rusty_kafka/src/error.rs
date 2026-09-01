use rusty_err::Error;

/// Errors from decoding a Kafka protocol message body. Distinct from
/// [`crate::ClientError`]: this is purely about malformed bytes, with no
/// notion of a network or a broker.
#[derive(Debug, Error)]
pub enum CodecError {
    /// Ran out of bytes, or a fixed-size read didn't fit -- bubbled up
    /// from [`rusty_wire::Reader`].
    #[error("buffer error: {0}")]
    Wire(#[from] rusty_wire::Error),
    /// A Kafka `STRING`/`NULLABLE_STRING` field's bytes weren't valid
    /// UTF-8.
    #[error("invalid UTF-8 in a Kafka STRING field")]
    InvalidUtf8,
    /// A Kafka array's `INT32` length prefix was below `-1` (`-1` means
    /// null; anything else negative is malformed).
    #[error("array length {0} is invalid (must be -1 or >= 0)")]
    InvalidArrayLength(i32),
    /// A Kafka string's `INT16` length prefix was below `-1`.
    #[error("string length {0} is invalid (must be -1 or >= 0)")]
    InvalidStringLength(i16),
    /// A record batch's `magic` byte wasn't `2` -- this crate only
    /// encodes/decodes record batch v2 (see
    /// [`crate::record_batch`]'s module doc for why).
    #[error("record batch magic byte {0} is not the supported value (2)")]
    UnsupportedMagic(u8),
}

/// Errors from talking to a Kafka broker over the wire: connection,
/// framing, and response-decoding failures, plus [`CodecError`] wrapped
/// in for any response body that fails to decode.
#[derive(Debug, Error)]
pub enum ClientError {
    /// The underlying connection failed (connect, read, or write).
    #[error("I/O error talking to the broker: {0}")]
    Io(#[from] std::io::Error),
    /// A response body failed to decode.
    #[error("failed to decode the broker's response: {0}")]
    Codec(#[from] CodecError),
    /// The broker declared a response frame larger than this client's
    /// configured cap -- rejected before allocating a buffer for it, so
    /// a corrupt or hostile length prefix can't force an unbounded
    /// allocation.
    #[error("broker declared a {1}-byte frame, over this client's {0}-byte limit")]
    FrameTooLarge(usize, u32),
    /// The response's `correlation_id` didn't match the request that
    /// was sent for it -- this client makes one request at a time per
    /// connection (see the crate's module doc), so any mismatch here
    /// means the connection's request/response stream has desynced and
    /// should be treated as unusable.
    #[error("response correlation_id {0} did not match the request's {1}")]
    CorrelationMismatch(i32, i32),
}

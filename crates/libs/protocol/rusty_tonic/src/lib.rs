//! The protocol core of gRPC, with no I/O: status codes and error details, validated metadata,
//! message framing, `grpc-timeout`, and the header lists of a request and a response.
//!
//! Nothing here is async or knows about HTTP/2. A transport hands in `(name, value)` header
//! pairs and bytes, and gets back typed values and header pairs to send. Services, streaming
//! and channels are built on top of this and live with the transport.
//!
//! Compression is not supported: a compressed message is `UNIMPLEMENTED`, and `identity` is
//! the only encoding.

#[cfg(feature = "rusty-tokio")]
pub mod client;
mod code;
mod frame;
mod head;
mod metadata;
#[cfg(feature = "rusty-tokio")]
pub mod server;
mod status;
mod timeout;
#[cfg(feature = "tonic-codec")]
pub mod tonic_codec;

pub use code::Code;
pub use frame::{
    decode_message, encode_frame, encode_message, FrameDecoder, DEFAULT_MAX_RECV_MESSAGE_SIZE,
    HEADER_LEN,
};
pub use head::{
    code_from_http_status, split_path, RequestHead, ResponseHead, ResponseStart, Scheme,
};
pub use metadata::{MetadataError, MetadataMap, ValueRef};
pub use status::Status;
pub use timeout::{format_timeout, parse_timeout, TimeoutError};

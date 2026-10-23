//! A `tonic` codec for `rusty_prost` messages, so a service keeps `tonic` as its transport while
//! its messages are written on `rusty_prost` and need no `protoc`.
//!
//! `WireCodec<T, U>` sends `T` and receives `U`. A message that fails to encode or decode is a
//! `tonic::Status` with code `Internal`.

use std::marker::PhantomData;

use bytes::{Buf, BufMut};
use rusty_prost::Message;
use tonic::codec::{BufferSettings, Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::Status;

/// A `tonic` codec for `rusty_prost` messages: `T` is sent, `U` is received.
#[derive(Debug, Clone)]
pub struct WireCodec<T, U>(PhantomData<(T, U)>);

impl<T, U> Default for WireCodec<T, U> {
    fn default() -> Self {
        WireCodec(PhantomData)
    }
}

impl<T: Message + Send + 'static, U: Message + Send + 'static> Codec for WireCodec<T, U> {
    type Encode = T;
    type Decode = U;
    type Encoder = WireEncoder<T>;
    type Decoder = WireDecoder<U>;

    fn encoder(&mut self) -> WireEncoder<T> {
        WireEncoder(PhantomData)
    }

    fn decoder(&mut self) -> WireDecoder<U> {
        WireDecoder(PhantomData)
    }
}

/// The sending half of [`WireCodec`].
#[derive(Debug, Clone)]
pub struct WireEncoder<T>(PhantomData<T>);

impl<T: Message> Encoder for WireEncoder<T> {
    type Item = T;
    type Error = Status;

    fn encode(&mut self, item: T, dst: &mut EncodeBuf<'_>) -> Result<(), Status> {
        let bytes = item
            .encode_to_vec()
            .map_err(|e| Status::internal(format!("failed to encode a message: {e}")))?;
        dst.put_slice(&bytes);
        Ok(())
    }

    fn buffer_settings(&self) -> BufferSettings {
        BufferSettings::default()
    }
}

/// The receiving half of [`WireCodec`].
#[derive(Debug, Clone)]
pub struct WireDecoder<U>(PhantomData<U>);

impl<U: Message> Decoder for WireDecoder<U> {
    type Item = U;
    type Error = Status;

    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<U>, Status> {
        let bytes = src.copy_to_bytes(src.remaining());
        U::decode(&bytes)
            .map(Some)
            .map_err(|e| Status::internal(format!("failed to decode a message: {e}")))
    }

    fn buffer_settings(&self) -> BufferSettings {
        BufferSettings::default()
    }
}

//! The request/response header every Kafka message carries, ahead of
//! its API-specific body.

use crate::error::CodecError;
use crate::wire::{read_i32, write_i16, write_i32, write_nullable_string};
use rusty_wire::{Reader, Writer};

/// A Kafka request header (v1): `api_key`, `api_version`,
/// `correlation_id`, `client_id`. Every request this crate sends uses
/// header v1 -- the version every "classic" (non-flexible) API version
/// pairs with once `client_id` was added to the protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestHeader {
    /// Which API this request invokes (see [`crate::protocol::api_key`]).
    pub api_key: i16,
    /// Which version of that API's request/response shape to use.
    pub api_version: i16,
    /// Echoed back verbatim in the matching [`ResponseHeader`]; how a
    /// client matches a response to the request that caused it.
    pub correlation_id: i32,
    /// An optional client identifier the broker may log/use for quotas.
    pub client_id: Option<String>,
}

impl RequestHeader {
    /// Encodes this header onto `writer`, ahead of the request body.
    pub fn encode(&self, writer: &mut Writer) {
        write_i16(writer, self.api_key);
        write_i16(writer, self.api_version);
        write_i32(writer, self.correlation_id);
        write_nullable_string(writer, self.client_id.as_deref());
    }
}

/// A Kafka response header (v0): just `correlation_id`, matching every
/// "classic" API version's response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponseHeader {
    /// Should equal the [`RequestHeader::correlation_id`] of the request
    /// this is a response to.
    pub correlation_id: i32,
}

impl ResponseHeader {
    /// Decodes a response header from the front of `reader`; the
    /// API-specific response body follows immediately after.
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        Ok(ResponseHeader {
            correlation_id: read_i32(reader)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_header_encodes_fields_in_order() {
        let header = RequestHeader {
            api_key: 19,
            api_version: 0,
            correlation_id: 7,
            client_id: Some("rusty_meshed".to_string()),
        };
        let mut writer = Writer::new();
        header.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        assert_eq!(crate::wire::read_i16(&mut reader).unwrap(), 19);
        assert_eq!(crate::wire::read_i16(&mut reader).unwrap(), 0);
        assert_eq!(read_i32(&mut reader).unwrap(), 7);
        assert_eq!(
            crate::wire::read_nullable_string(&mut reader).unwrap(),
            Some("rusty_meshed".to_string())
        );
    }

    #[test]
    fn request_header_encodes_null_client_id() {
        let header = RequestHeader {
            api_key: 3,
            api_version: 0,
            correlation_id: 1,
            client_id: None,
        };
        let mut writer = Writer::new();
        header.encode(&mut writer);
        let bytes = writer.into_vec();
        // api_key(2) + api_version(2) + correlation_id(4) + client_id len -1(2)
        assert_eq!(bytes.len(), 10);
        assert_eq!(&bytes[8..10], [0xFF, 0xFF]);
    }

    #[test]
    fn response_header_decodes_correlation_id() {
        let mut writer = Writer::new();
        write_i32(&mut writer, 42);
        let bytes = writer.into_vec();
        let mut reader = Reader::new(&bytes);
        assert_eq!(
            ResponseHeader::decode(&mut reader).unwrap(),
            ResponseHeader { correlation_id: 42 }
        );
    }
}

//! `ApiVersions` (API key 18) v0: an empty-bodied request that asks the
//! broker which API keys it supports and at which version range for
//! each -- the request every well-behaved client sends first, so it can
//! adapt to what the specific broker it's talking to actually accepts
//! rather than assuming a version blindly.

use crate::error::CodecError;
use crate::wire::{read_array_len, read_i16};
use rusty_wire::{Reader, Writer};

/// `ApiVersionsRequest` v0 -- no body fields; the header alone is the
/// whole request.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApiVersionsRequest;

impl ApiVersionsRequest {
    /// Encodes the (empty) v0 body. Present for symmetry with every
    /// other request type in this crate, and so a version bump that
    /// adds body fields later doesn't change the call site.
    pub fn encode(&self, _writer: &mut Writer) {}
}

/// One API key's supported version range, as reported by a broker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiVersion {
    /// The API key this entry describes (see [`crate::protocol::api_key`]).
    pub api_key: i16,
    /// Lowest version of that API the broker accepts.
    pub min_version: i16,
    /// Highest version of that API the broker accepts.
    pub max_version: i16,
}

/// `ApiVersionsResponse` v0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiVersionsResponse {
    /// Kafka error code; `0` means success.
    pub error_code: i16,
    /// One entry per API key the broker supports.
    pub api_versions: Vec<ApiVersion>,
}

impl ApiVersionsResponse {
    /// Decodes the response body (the [`crate::protocol::header::ResponseHeader`]
    /// is already consumed by the caller).
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        let error_code = read_i16(reader)?;
        let count = read_array_len(reader)?.max(0);
        let mut api_versions = Vec::with_capacity(count as usize);
        for _ in 0..count {
            api_versions.push(ApiVersion {
                api_key: read_i16(reader)?,
                min_version: read_i16(reader)?,
                max_version: read_i16(reader)?,
            });
        }
        Ok(ApiVersionsResponse {
            error_code,
            api_versions,
        })
    }

    /// The broker-supported version range for `api_key`, if it
    /// advertised one.
    pub fn range_for(&self, api_key: i16) -> Option<(i16, i16)> {
        self.api_versions
            .iter()
            .find(|entry| entry.api_key == api_key)
            .map(|entry| (entry.min_version, entry.max_version))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::write_i16;

    fn encode_response(error_code: i16, versions: &[(i16, i16, i16)]) -> Vec<u8> {
        let mut writer = Writer::new();
        write_i16(&mut writer, error_code);
        crate::wire::write_i32(&mut writer, versions.len() as i32);
        for (api_key, min_version, max_version) in versions {
            write_i16(&mut writer, *api_key);
            write_i16(&mut writer, *min_version);
            write_i16(&mut writer, *max_version);
        }
        writer.into_vec()
    }

    #[test]
    fn decodes_empty_response() {
        let bytes = encode_response(0, &[]);
        let mut reader = Reader::new(&bytes);
        let response = ApiVersionsResponse::decode(&mut reader).unwrap();
        assert_eq!(
            response,
            ApiVersionsResponse {
                error_code: 0,
                api_versions: vec![]
            }
        );
    }

    #[test]
    fn decodes_multiple_api_versions() {
        let bytes = encode_response(0, &[(18, 0, 3), (19, 0, 7), (3, 0, 12)]);
        let mut reader = Reader::new(&bytes);
        let response = ApiVersionsResponse::decode(&mut reader).unwrap();
        assert_eq!(response.api_versions.len(), 3);
        assert_eq!(response.range_for(19), Some((0, 7)));
        assert_eq!(response.range_for(0), None);
    }

    #[test]
    fn decodes_nonzero_error_code() {
        let bytes = encode_response(35, &[]);
        let mut reader = Reader::new(&bytes);
        let response = ApiVersionsResponse::decode(&mut reader).unwrap();
        assert_eq!(response.error_code, 35);
    }
}

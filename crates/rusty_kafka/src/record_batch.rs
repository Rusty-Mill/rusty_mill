//! Kafka record batch v2 (`magic = 2`) encoding -- the wire format
//! every `Produce` request version 3+ requires for its `records` bytes
//! (KIP-98). Independent of whether the *request* itself uses classic
//! or flexible (compact/tagged-field) encoding -- record batch v2 was
//! fixed by KIP-98 in 2017, well before KIP-482's flexible versions
//! existed, so its own internal varint fields are always the plain
//! zigzag scheme below, never the "compact" unsigned-plus-one scheme
//! flexible request/response bodies use elsewhere.
//!
//! This crate has no live Kafka broker to validate wire-format code
//! against (see the crate's own module doc), so every piece here is
//! hand-verified instead: CRC-32C against the standard Castagnoli test
//! vector (`"123456789"` -> `0xE3069283`), and the batch/record layout
//! against the published field-by-field spec, with round-trip tests
//! through this module's own decoder (`decode_batch`, built and tested
//! specifically to cross-check `encode_batch`, not needed by any real
//! caller -- `ProduceResponse` never returns record batch bytes).
//!
//! # Layout
//!
//! ```text
//! RecordBatch:
//!   baseOffset            int64   (always 0 -- producer-assigned batches)
//!   batchLength           int32   (bytes after this field to the end of the batch)
//!   partitionLeaderEpoch  int32   (-1, producer doesn't know/set it)
//!   magic                 int8    (2)
//!   crc                   int32   (CRC-32C of everything after this field)
//!   attributes            int16   (0: no compression, CreateTime, non-transactional)
//!   lastOffsetDelta       int32   (recordsCount - 1)
//!   baseTimestamp         int64
//!   maxTimestamp          int64   (== baseTimestamp: every record shares one timestamp)
//!   producerId            int64   (-1: non-idempotent)
//!   producerEpoch         int16   (-1)
//!   baseSequence          int32   (-1)
//!   recordsCount          int32
//!   records               [Record]
//!
//! Record (each prefixed by its own varint length, itself excluded from that length):
//!   length          varint
//!   attributes      int8    (0, unused)
//!   timestampDelta  varint  (varlong; 0 for every record here -- one shared timestamp)
//!   offsetDelta     varint  (this record's 0-based index in the batch)
//!   keyLength       varint  (-1 = null)
//!   key             <keyLength> bytes
//!   valueLength     varint  (-1 = null)
//!   value           <valueLength> bytes
//!   headersCount    varint
//!   headers:
//!     headerKeyLength    varint (never null)
//!     headerKey          <headerKeyLength> bytes
//!     headerValueLength  varint (-1 = null)
//!     headerValue        <headerValueLength> bytes
//! ```

use crate::error::CodecError;
use crate::wire::{read_i16, read_i32, read_i64, write_i16, write_i32, write_i64};
use rusty_wire::{Reader, Writer};

/// One record to encode into a [`encode_batch`] call -- a
/// produce-side analogue of `KafkaClient`'s other protocol request
/// types, but living in this module since it's specific to record
/// batch encoding rather than one request/response pair.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Record {
    pub key: Option<Vec<u8>>,
    pub value: Option<Vec<u8>>,
    /// Kafka message headers: key (never null) plus an optional value.
    pub headers: Vec<(String, Option<Vec<u8>>)>,
}

/// Encodes `records` as a single record batch v2, every record sharing
/// `base_timestamp_ms` (milliseconds since the Unix epoch) as its
/// timestamp.
pub fn encode_batch(records: &[Record], base_timestamp_ms: i64) -> Vec<u8> {
    let mut records_body = Writer::new();
    for (index, record) in records.iter().enumerate() {
        encode_record(&mut records_body, record, index as i32, 0);
    }
    let records_body = records_body.into_vec();

    let mut crc_input = Writer::new();
    write_i16(&mut crc_input, 0); // attributes: no compression, CreateTime, non-transactional
    write_i32(&mut crc_input, records.len() as i32 - 1); // lastOffsetDelta
    write_i64(&mut crc_input, base_timestamp_ms); // baseTimestamp
    write_i64(&mut crc_input, base_timestamp_ms); // maxTimestamp (one shared timestamp)
    write_i64(&mut crc_input, -1); // producerId: non-idempotent
    write_i16(&mut crc_input, -1); // producerEpoch
    write_i32(&mut crc_input, -1); // baseSequence
    write_i32(&mut crc_input, records.len() as i32); // recordsCount
    crc_input.write_bytes(&records_body);
    let crc_input = crc_input.into_vec();
    let crc = crc32c(&crc_input);

    let mut out = Writer::new();
    write_i64(&mut out, 0); // baseOffset
    write_i32(&mut out, (4 + 1 + 4 + crc_input.len()) as i32); // batchLength
    write_i32(&mut out, -1); // partitionLeaderEpoch
    out.write_u8(2); // magic
    write_i32(&mut out, crc as i32);
    out.write_bytes(&crc_input);
    out.into_vec()
}

fn encode_record(writer: &mut Writer, record: &Record, offset_delta: i32, timestamp_delta: i64) {
    let mut body = Writer::new();
    body.write_u8(0); // attributes, unused
    write_varlong(&mut body, timestamp_delta);
    write_varint(&mut body, offset_delta);
    write_bytes_field(&mut body, record.key.as_deref());
    write_bytes_field(&mut body, record.value.as_deref());
    write_varint(&mut body, record.headers.len() as i32);
    for (key, value) in &record.headers {
        write_varint(&mut body, key.len() as i32);
        body.write_bytes(key.as_bytes());
        write_bytes_field(&mut body, value.as_deref());
    }
    let body = body.into_vec();
    write_varint(writer, body.len() as i32);
    writer.write_bytes(&body);
}

fn write_bytes_field(writer: &mut Writer, value: Option<&[u8]>) {
    match value {
        None => write_varint(writer, -1),
        Some(bytes) => {
            write_varint(writer, bytes.len() as i32);
            writer.write_bytes(bytes);
        }
    }
}

/// Decodes a record batch v2 built by [`encode_batch`] -- exists so
/// this module's own tests can round-trip and hand-verify the format,
/// not because any real caller needs it (`ProduceResponse` never
/// returns record batch bytes).
pub fn decode_batch(bytes: &[u8]) -> Result<Vec<Record>, CodecError> {
    let mut reader = Reader::new(bytes);
    let _base_offset = read_i64(&mut reader)?;
    let batch_length = read_i32(&mut reader)?;
    if batch_length < 0 {
        return Err(CodecError::InvalidArrayLength(batch_length));
    }
    let _partition_leader_epoch = read_i32(&mut reader)?;
    let magic = reader.read_u8()?;
    if magic != 2 {
        return Err(CodecError::UnsupportedMagic(magic));
    }
    let _crc = read_i32(&mut reader)?;
    let _attributes = read_i16(&mut reader)?;
    let _last_offset_delta = read_i32(&mut reader)?;
    let _base_timestamp = read_i64(&mut reader)?;
    let _max_timestamp = read_i64(&mut reader)?;
    let _producer_id = read_i64(&mut reader)?;
    let _producer_epoch = read_i16(&mut reader)?;
    let _base_sequence = read_i32(&mut reader)?;
    let records_count = read_i32(&mut reader)?;
    if records_count < 0 {
        return Err(CodecError::InvalidArrayLength(records_count));
    }

    let mut records = Vec::with_capacity(records_count as usize);
    for _ in 0..records_count {
        records.push(decode_record(&mut reader)?);
    }
    Ok(records)
}

fn decode_record(reader: &mut Reader) -> Result<Record, CodecError> {
    let _length = read_varint(reader)?;
    let _attributes = reader.read_u8()?;
    let _timestamp_delta = read_varlong(reader)?;
    let _offset_delta = read_varint(reader)?;
    let key = read_bytes_field(reader)?;
    let value = read_bytes_field(reader)?;
    let header_count = read_varint(reader)?;
    if header_count < 0 {
        return Err(CodecError::InvalidArrayLength(header_count));
    }
    let mut headers = Vec::with_capacity(header_count as usize);
    for _ in 0..header_count {
        let key_len = read_varint(reader)?;
        if key_len < 0 {
            return Err(CodecError::InvalidArrayLength(key_len));
        }
        let key_bytes = reader.read_bytes(key_len as usize)?;
        let header_key = std::str::from_utf8(key_bytes)
            .map_err(|_| CodecError::InvalidUtf8)?
            .to_string();
        let header_value = read_bytes_field(reader)?;
        headers.push((header_key, header_value));
    }
    Ok(Record {
        key,
        value,
        headers,
    })
}

fn read_bytes_field(reader: &mut Reader) -> Result<Option<Vec<u8>>, CodecError> {
    let len = read_varint(reader)?;
    if len < -1 {
        return Err(CodecError::InvalidArrayLength(len));
    }
    if len == -1 {
        return Ok(None);
    }
    Ok(Some(reader.read_bytes(len as usize)?.to_vec()))
}

// ---------------------------------------------------------------------
// Varint/varlong: zigzag then variable-length base-128, matching
// Protocol Buffers' scheme (Kafka's own documented choice) -- distinct
// from this crate's classic-protocol fixed-width INT16/INT32/INT64
// helpers in `wire.rs`, and only ever used inside record batch bytes.
// ---------------------------------------------------------------------

fn write_varlong(writer: &mut Writer, value: i64) {
    let mut n = ((value << 1) ^ (value >> 63)) as u64;
    loop {
        let byte = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            writer.write_u8(byte);
            break;
        }
        writer.write_u8(byte | 0x80);
    }
}

fn read_varlong(reader: &mut Reader) -> Result<i64, CodecError> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    loop {
        let byte = reader.read_u8()?;
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    Ok(((result >> 1) as i64) ^ -((result & 1) as i64))
}

fn write_varint(writer: &mut Writer, value: i32) {
    write_varlong(writer, value as i64);
}

fn read_varint(reader: &mut Reader) -> Result<i32, CodecError> {
    let value = read_varlong(reader)?;
    Ok(value as i32)
}

// ---------------------------------------------------------------------
// CRC-32C (Castagnoli) -- what the record batch `crc` field uses,
// distinct from the standard CRC-32/ISO-HDLC (zlib/gzip) polynomial.
// Table-based, same shape as `rusty_stream::record`'s CRC-32
// implementation, just the Castagnoli reflected polynomial instead.
// ---------------------------------------------------------------------

const CRC32C_POLY: u32 = 0x82F6_3B78;

fn crc32c_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut j = 0;
        while j < 8 {
            c = if c & 1 != 0 {
                CRC32C_POLY ^ (c >> 1)
            } else {
                c >> 1
            };
            j += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

/// CRC-32C (Castagnoli) checksum of `data`.
fn crc32c(data: &[u8]) -> u32 {
    let table = crc32c_table();
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        let idx = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = table[idx] ^ (crc >> 8);
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32c_matches_the_standard_castagnoli_test_vector() {
        // The official CRC-32C (iSCSI) test vector.
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }

    #[test]
    fn varint_round_trips_negative_zero_and_positive_i32_values() {
        for value in [i32::MIN, -1_000_000, -1, 0, 1, 1_000_000, i32::MAX] {
            let mut writer = Writer::new();
            write_varint(&mut writer, value);
            let bytes = writer.into_vec();
            let mut reader = Reader::new(&bytes);
            assert_eq!(read_varint(&mut reader).unwrap(), value);
            assert!(reader.is_empty());
        }
    }

    #[test]
    fn varlong_round_trips_negative_zero_and_positive_i64_values() {
        for value in [i64::MIN, -1_000_000, -1, 0, 1, 1_000_000, i64::MAX] {
            let mut writer = Writer::new();
            write_varlong(&mut writer, value);
            let bytes = writer.into_vec();
            let mut reader = Reader::new(&bytes);
            assert_eq!(read_varlong(&mut reader).unwrap(), value);
            assert!(reader.is_empty());
        }
    }

    #[test]
    fn single_record_batch_round_trips_key_value_and_headers() {
        let records = vec![Record {
            key: Some(b"order-42".to_vec()),
            value: Some(b"{\"status\":\"created\"}".to_vec()),
            headers: vec![
                ("event_id".to_string(), Some(b"e-1".to_vec())),
                ("correlation_id".to_string(), Some(b"c-1".to_vec())),
            ],
        }];
        let bytes = encode_batch(&records, 1_735_689_600_000);
        let decoded = decode_batch(&bytes).unwrap();
        assert_eq!(decoded, records);
    }

    #[test]
    fn record_with_a_null_key_round_trips() {
        let records = vec![Record {
            key: None,
            value: Some(b"value-only".to_vec()),
            headers: vec![],
        }];
        let bytes = encode_batch(&records, 0);
        let decoded = decode_batch(&bytes).unwrap();
        assert_eq!(decoded, records);
    }

    #[test]
    fn record_with_a_null_header_value_round_trips() {
        let records = vec![Record {
            key: None,
            value: None,
            headers: vec![("flag".to_string(), None)],
        }];
        let bytes = encode_batch(&records, 0);
        let decoded = decode_batch(&bytes).unwrap();
        assert_eq!(decoded, records);
    }

    #[test]
    fn multiple_records_in_one_batch_round_trip_with_correct_offsets() {
        let records = vec![
            Record {
                key: Some(b"k1".to_vec()),
                value: Some(b"v1".to_vec()),
                headers: vec![],
            },
            Record {
                key: Some(b"k2".to_vec()),
                value: Some(b"v2".to_vec()),
                headers: vec![],
            },
            Record {
                key: Some(b"k3".to_vec()),
                value: Some(b"v3".to_vec()),
                headers: vec![],
            },
        ];
        let bytes = encode_batch(&records, 1_735_689_600_000);
        let decoded = decode_batch(&bytes).unwrap();
        assert_eq!(decoded, records);
    }

    #[test]
    fn magic_byte_is_2() {
        let records = vec![Record::default()];
        let bytes = encode_batch(&records, 0);
        // baseOffset(8) + batchLength(4) + partitionLeaderEpoch(4) = byte 16 is magic.
        assert_eq!(bytes[16], 2);
    }

    #[test]
    fn batch_length_matches_the_actual_encoded_size() {
        let records = vec![Record {
            key: Some(b"k".to_vec()),
            value: Some(b"v".to_vec()),
            headers: vec![],
        }];
        let bytes = encode_batch(&records, 0);
        let mut reader = Reader::new(&bytes);
        let _base_offset = read_i64(&mut reader).unwrap();
        let batch_length = read_i32(&mut reader).unwrap();
        // batchLength covers everything after itself to the end of the batch.
        assert_eq!(batch_length as usize, bytes.len() - 8 - 4);
    }

    #[test]
    fn corrupting_a_record_byte_is_detected_by_a_fresh_crc_check() {
        let records = vec![Record {
            key: Some(b"k".to_vec()),
            value: Some(b"v".to_vec()),
            headers: vec![],
        }];
        let bytes = encode_batch(&records, 0);
        // The CRC field itself is at offset 8(baseOffset)+4(batchLength)+4(partitionLeaderEpoch)+1(magic) = 17.
        let stored_crc = {
            let mut reader = Reader::new(&bytes[17..]);
            read_i32(&mut reader).unwrap() as u32
        };
        let recomputed = crc32c(&bytes[21..]);
        assert_eq!(stored_crc, recomputed);

        let mut corrupted = bytes.clone();
        let last = corrupted.len() - 1;
        corrupted[last] ^= 0xFF;
        let recomputed_after_corruption = crc32c(&corrupted[21..]);
        assert_ne!(stored_crc, recomputed_after_corruption);
    }
}

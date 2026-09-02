//! `Fetch` (API key 1) v4: pulls records from a topic/partition
//! starting at a given offset -- the read side matching
//! [`crate::protocol::produce`]'s write side.
//!
//! v4, not v0 -- same reasoning [`crate::protocol::produce`]'s own
//! module doc gives for its own version choice: message format v2
//! (record batch `magic = 2`, [`crate::record_batch`]) isn't returned
//! to a `FetchRequest` older than v4 -- a broker transparently
//! down-converts to the legacy v0/v1 message-set format for anything
//! below that, which this crate doesn't decode. v4 also adds
//! `isolation_level` and, on the response, `last_stable_offset`/
//! `aborted_transactions` (both there to support transactional reads)
//! -- this crate has no producer-side transaction support either (see
//! `produce`'s own module doc: `transactional_id` is always sent
//! null), so `isolation_level` is always `READ_UNCOMMITTED` and
//! `aborted_transactions` is decoded for wire correctness but never
//! acted on.

use crate::error::CodecError;
use crate::record_batch::{self, Record};
use crate::wire::{
    read_array_len, read_i16, read_i32, read_i64, read_i8, read_nullable_bytes, read_string,
    write_i16, write_i32, write_i64, write_i8, write_string,
};
use rusty_wire::{Reader, Writer};

/// `READ_UNCOMMITTED` -- the only isolation level this crate's
/// producer side can actually produce under (no transactions), so the
/// only one worth naming here.
pub const READ_UNCOMMITTED: i8 = 0;

/// One partition to fetch within a [`FetchTopicRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchPartitionRequest {
    pub partition_index: i32,
    /// The offset to start fetching from -- the next offset this
    /// consumer wants to read.
    pub fetch_offset: i64,
    /// Caps how many bytes this one partition's response may
    /// contribute towards [`FetchRequest::max_bytes`].
    pub partition_max_bytes: i32,
}

/// One topic's partitions to fetch within a [`FetchRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchTopicRequest {
    pub name: String,
    pub partitions: Vec<FetchPartitionRequest>,
}

/// `FetchRequest` v4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchRequest {
    /// `-1` for a regular consumer (not a broker replica).
    pub replica_id: i32,
    /// Maximum time the broker will block waiting for
    /// [`min_bytes`](Self::min_bytes) to accumulate before replying
    /// anyway, even with fewer bytes (or none).
    pub max_wait_ms: i32,
    /// Minimum bytes the broker should try to accumulate across the
    /// whole response before replying (subject to
    /// [`max_wait_ms`](Self::max_wait_ms)).
    pub min_bytes: i32,
    /// Caps the whole response's total byte size.
    pub max_bytes: i32,
    /// Always [`READ_UNCOMMITTED`] in practice -- see the module doc.
    pub isolation_level: i8,
    pub topics: Vec<FetchTopicRequest>,
}

impl Default for FetchRequest {
    fn default() -> Self {
        FetchRequest {
            replica_id: -1,
            max_wait_ms: 0,
            min_bytes: 0,
            max_bytes: 0,
            isolation_level: READ_UNCOMMITTED,
            topics: Vec::new(),
        }
    }
}

impl FetchRequest {
    /// Encodes the v4 body.
    pub fn encode(&self, writer: &mut Writer) {
        write_i32(writer, self.replica_id);
        write_i32(writer, self.max_wait_ms);
        write_i32(writer, self.min_bytes);
        write_i32(writer, self.max_bytes);
        write_i8(writer, self.isolation_level);
        write_i32(writer, self.topics.len() as i32);
        for topic in &self.topics {
            write_string(writer, &topic.name);
            write_i32(writer, topic.partitions.len() as i32);
            for partition in &topic.partitions {
                write_i32(writer, partition.partition_index);
                write_i64(writer, partition.fetch_offset);
                write_i32(writer, partition.partition_max_bytes);
            }
        }
    }

    /// Decodes a v4 body -- symmetric with [`encode`](Self::encode),
    /// for a fake broker standing in for tests (see
    /// [`crate::testing`]; this crate is client-only).
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        let replica_id = read_i32(reader)?;
        let max_wait_ms = read_i32(reader)?;
        let min_bytes = read_i32(reader)?;
        let max_bytes = read_i32(reader)?;
        let isolation_level = read_i8(reader)?;
        let topic_count = read_array_len(reader)?.max(0);
        let mut topics = Vec::with_capacity(topic_count as usize);
        for _ in 0..topic_count {
            let name = read_string(reader)?;
            let partition_count = read_array_len(reader)?.max(0);
            let mut partitions = Vec::with_capacity(partition_count as usize);
            for _ in 0..partition_count {
                partitions.push(FetchPartitionRequest {
                    partition_index: read_i32(reader)?,
                    fetch_offset: read_i64(reader)?,
                    partition_max_bytes: read_i32(reader)?,
                });
            }
            topics.push(FetchTopicRequest { name, partitions });
        }
        Ok(FetchRequest {
            replica_id,
            max_wait_ms,
            min_bytes,
            max_bytes,
            isolation_level,
            topics,
        })
    }
}

/// One aborted transaction within a [`FetchPartitionResponse`] --
/// decoded for wire correctness (v4 always sends this array, possibly
/// empty) but never acted on, since this crate has no transactional
/// producer to need read-committed filtering for (see the module
/// doc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbortedTransaction {
    pub producer_id: i64,
    pub first_offset: i64,
}

/// One partition's fetched records within a [`FetchTopicResponse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchPartitionResponse {
    pub partition_index: i32,
    /// Kafka error code; `0` means success.
    pub error_code: i16,
    /// The partition's current high-watermark offset.
    pub high_watermark: i64,
    /// The highest offset a consumer under `READ_COMMITTED` isolation
    /// may read up to -- meaningless under [`READ_UNCOMMITTED`] (the
    /// only isolation level this crate ever requests), included only
    /// because v4 always sends it.
    pub last_stable_offset: i64,
    pub aborted_transactions: Vec<AbortedTransaction>,
    /// Every record decoded from this partition's record batch(es),
    /// via [`record_batch::decode_batch`] -- empty if the partition
    /// had nothing new past the requested offset.
    pub records: Vec<Record>,
}

/// One topic's results within a [`FetchResponse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchTopicResponse {
    pub name: String,
    pub partitions: Vec<FetchPartitionResponse>,
}

/// `FetchResponse` v4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchResponse {
    pub throttle_time_ms: i32,
    pub topics: Vec<FetchTopicResponse>,
}

impl FetchResponse {
    /// Decodes the response body.
    pub fn decode(reader: &mut Reader) -> Result<Self, CodecError> {
        let throttle_time_ms = read_i32(reader)?;
        let topic_count = read_array_len(reader)?.max(0);
        let mut topics = Vec::with_capacity(topic_count as usize);
        for _ in 0..topic_count {
            let name = read_string(reader)?;
            let partition_count = read_array_len(reader)?.max(0);
            let mut partitions = Vec::with_capacity(partition_count as usize);
            for _ in 0..partition_count {
                let partition_index = read_i32(reader)?;
                let error_code = read_i16(reader)?;
                let high_watermark = read_i64(reader)?;
                let last_stable_offset = read_i64(reader)?;
                let aborted_count = read_array_len(reader)?.max(0);
                let mut aborted_transactions = Vec::with_capacity(aborted_count as usize);
                for _ in 0..aborted_count {
                    aborted_transactions.push(AbortedTransaction {
                        producer_id: read_i64(reader)?,
                        first_offset: read_i64(reader)?,
                    });
                }
                let records = match read_nullable_bytes(reader)? {
                    None | Some([]) => Vec::new(),
                    Some(bytes) => record_batch::decode_batch(bytes)?,
                };
                partitions.push(FetchPartitionResponse {
                    partition_index,
                    error_code,
                    high_watermark,
                    last_stable_offset,
                    aborted_transactions,
                    records,
                });
            }
            topics.push(FetchTopicResponse { name, partitions });
        }
        Ok(FetchResponse {
            throttle_time_ms,
            topics,
        })
    }

    /// Encodes the response body -- symmetric with
    /// [`decode`](Self::decode), for a fake broker standing in for
    /// tests. `base_timestamp_ms` picks the timestamp
    /// [`record_batch::encode_batch`] stamps every encoded record
    /// with (this crate's fake broker never needs per-record
    /// timestamps to vary).
    pub fn encode(&self, writer: &mut Writer, base_timestamp_ms: i64) {
        write_i32(writer, self.throttle_time_ms);
        write_i32(writer, self.topics.len() as i32);
        for topic in &self.topics {
            write_string(writer, &topic.name);
            write_i32(writer, topic.partitions.len() as i32);
            for partition in &topic.partitions {
                write_i32(writer, partition.partition_index);
                write_i16(writer, partition.error_code);
                write_i64(writer, partition.high_watermark);
                write_i64(writer, partition.last_stable_offset);
                write_i32(writer, partition.aborted_transactions.len() as i32);
                for aborted in &partition.aborted_transactions {
                    write_i64(writer, aborted.producer_id);
                    write_i64(writer, aborted.first_offset);
                }
                if partition.records.is_empty() {
                    write_i32(writer, -1);
                } else {
                    let batch = record_batch::encode_batch(&partition.records, base_timestamp_ms);
                    write_i32(writer, batch.len() as i32);
                    writer.write_bytes(&batch);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> FetchRequest {
        FetchRequest {
            replica_id: -1,
            max_wait_ms: 1000,
            min_bytes: 1,
            max_bytes: 1_048_576,
            isolation_level: READ_UNCOMMITTED,
            topics: vec![FetchTopicRequest {
                name: "manpower.personnel-lifecycle.assignments".to_string(),
                partitions: vec![FetchPartitionRequest {
                    partition_index: 0,
                    fetch_offset: 5,
                    partition_max_bytes: 1_048_576,
                }],
            }],
        }
    }

    #[test]
    fn request_encode_then_decode_round_trips() {
        let request = sample_request();
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        assert_eq!(FetchRequest::decode(&mut reader).unwrap(), request);
    }

    #[test]
    fn request_encodes_replica_id_negative_one_for_a_consumer() {
        let request = sample_request();
        let mut writer = Writer::new();
        request.encode(&mut writer);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        assert_eq!(crate::wire::read_i32(&mut reader).unwrap(), -1);
    }

    #[test]
    fn response_round_trips_fetched_records() {
        let response = FetchResponse {
            throttle_time_ms: 0,
            topics: vec![FetchTopicResponse {
                name: "manpower.personnel-lifecycle.assignments".to_string(),
                partitions: vec![FetchPartitionResponse {
                    partition_index: 0,
                    error_code: 0,
                    high_watermark: 6,
                    last_stable_offset: 6,
                    aborted_transactions: vec![],
                    records: vec![Record {
                        key: None,
                        value: Some(b"{\"person_id\":\"p-1\"}".to_vec()),
                        headers: vec![("event_id".to_string(), Some(b"e-1".to_vec()))],
                    }],
                }],
            }],
        };
        let mut writer = Writer::new();
        response.encode(&mut writer, 1_735_689_600_000);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        let decoded = FetchResponse::decode(&mut reader).unwrap();
        assert_eq!(decoded.topics[0].partitions[0].high_watermark, 6);
        assert_eq!(decoded.topics[0].partitions[0].records.len(), 1);
        assert_eq!(
            decoded.topics[0].partitions[0].records[0].value,
            Some(b"{\"person_id\":\"p-1\"}".to_vec())
        );
    }

    #[test]
    fn response_treats_a_null_record_set_as_no_records() {
        let response = FetchResponse {
            throttle_time_ms: 0,
            topics: vec![FetchTopicResponse {
                name: "t".to_string(),
                partitions: vec![FetchPartitionResponse {
                    partition_index: 0,
                    error_code: 0,
                    high_watermark: 0,
                    last_stable_offset: 0,
                    aborted_transactions: vec![],
                    records: vec![],
                }],
            }],
        };
        let mut writer = Writer::new();
        response.encode(&mut writer, 0);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        let decoded = FetchResponse::decode(&mut reader).unwrap();
        assert!(decoded.topics[0].partitions[0].records.is_empty());
    }

    #[test]
    fn response_decodes_a_broker_error() {
        let response = FetchResponse {
            throttle_time_ms: 0,
            topics: vec![FetchTopicResponse {
                name: "no-such-topic".to_string(),
                partitions: vec![FetchPartitionResponse {
                    partition_index: 0,
                    error_code: 3, // UNKNOWN_TOPIC_OR_PARTITION
                    high_watermark: -1,
                    last_stable_offset: -1,
                    aborted_transactions: vec![],
                    records: vec![],
                }],
            }],
        };
        let mut writer = Writer::new();
        response.encode(&mut writer, 0);
        let bytes = writer.into_vec();

        let mut reader = Reader::new(&bytes);
        let decoded = FetchResponse::decode(&mut reader).unwrap();
        assert_eq!(decoded.topics[0].partitions[0].error_code, 3);
    }
}

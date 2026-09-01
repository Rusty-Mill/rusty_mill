//! [`DataProductConsumerBase`] -- the Rust port of
//! `meshed.sdk.consumer.DataProductConsumerBase` (SDK-029..042).
//!
//! # The `rusty_kafka` gap this pass closes
//!
//! `startup()`'s Steps 3-5 (SDK-034..036) and the poll loop (SDK-039)
//! needed `rusty_kafka` to fetch messages and manage consumer-group
//! offsets -- `FindCoordinator`/`JoinGroup`/`SyncGroup`/`Heartbeat`/
//! `OffsetCommit`/`Fetch`. That landed as its own `rusty_kafka` pass
//! (GitHub issue #91's own framing: "if it's bigger than this issue
//! should carry, split it into its own `rusty_kafka` follow-up issue
//! first"). This module is the follow-through: building
//! `startup()`/`run()`/`stop()` on top of those primitives.
//!
//! # No `DeserializingConsumer`, no confluent-kafka-managed rebalance
//!
//! The source hands `group.id`/`auto.offset.reset`/
//! `enable.auto.commit=False`/deserializers to a `DeserializingConsumer`
//! and lets `confluent_kafka`'s underlying `librdkafka` drive consumer-
//! group membership, rebalancing, and internal message prefetching.
//! There is no such library here -- this port drives the group
//! membership protocol directly:
//!
//! - **Join and sync** happen once, in [`startup`](DataProductConsumerBase::startup),
//!   using an always-empty initial `member_id` (a fresh join every
//!   `startup()` call, never a rejoin with a stale ID).
//! - **Partition assignment**, when this member is elected leader, is
//!   this crate's own policy -- [`crate::assignor::range_assign`],
//!   mirroring Kafka's built-in `"range"` assignor (the same
//!   `protocol_name` this crate declares) -- since `rusty_kafka`
//!   explicitly leaves that decision to its callers (see that crate's
//!   own module doc).
//! - **Heartbeats and fetches share rusty_kafka's one connection**
//!   (`rusty_kafka::KafkaClient` is single-connection, no pipelining --
//!   see that crate's own module doc), so [`run`](DataProductConsumerBase::run)
//!   sends one `Heartbeat` immediately before each `Fetch` rather than
//!   on an independent background timer, the same "heartbeat tied to
//!   the poll loop" design pre-0.10.1 Kafka consumers used. This is
//!   safe as long as one loop iteration (heartbeat + fetch + however
//!   many fetched records' `process()`/commit calls) comfortably fits
//!   inside [`SESSION_TIMEOUT_MS`] -- true for this platform's small,
//!   per-request batch sizes, but a real limitation worth stating
//!   plainly: a slow `process()` callback can still starve the
//!   coordinator of heartbeats and trigger a rebalance.
//! - **Automatic rebalance handling is not implemented.** Neither is
//!   it in the source: `_poll_loop` has no rebalance-related code at
//!   all (confluent-kafka's default `on_assign`/`on_revoke` behavior,
//!   invisible in the Python source, is what actually handles it
//!   there). A `REBALANCE_IN_PROGRESS` (or any other) `Heartbeat`/
//!   `Fetch` error surfaces as a fatal [`ConsumerRunError`], ending
//!   [`run`](DataProductConsumerBase::run) -- reasonable for this
//!   platform's one-instance-per-group deployment, where nothing else
//!   ever joins the group to trigger a rebalance in the first place.
//! - **One `Fetch` returns a batch; `process()`/commit still happen
//!   one record at a time**, in fetch order, exactly mirroring the
//!   source's own per-message `poll()`/`process()`/`commit()` loop --
//!   confluent-kafka's `poll()` also returns one message at a time, via
//!   its own internal prefetch buffer; this port's "buffer" is just an
//!   explicit `Fetch` response instead of a library-internal one, with
//!   no behavioral difference at the message-processing level: skip
//!   duplicates, run `process()`, commit only after it succeeds, before
//!   moving to the next record.
//!
//! # The Kafka connection is established at construction, not `startup()`
//!
//! Unlike the source's `self._consumer: DeserializingConsumer | None`
//! (only built in `startup()`), [`connect`](DataProductConsumerBase::connect)
//! opens the `KafkaClient` connection eagerly, the same choice
//! `DataProductProducerBase::connect` already made for its own Kafka
//! clients (see that module's own doc). `startup()` still does the
//! group-membership work (join/sync/resolve starting offsets) that
//! Python's `startup()` did via `subscribe()`.
//!
//! # `run()`/`stop()`: a stop handle, not a shared `self._running` flag
//!
//! The source's `run()` blocks a `ThreadPoolExecutor` worker thread on
//! `_poll_loop` while `stop()` is called from a *different* thread (or
//! coroutine) sharing the same `self`. Rust's borrow checker forbids
//! that directly: [`run`](DataProductConsumerBase::run) holds `&mut
//! self` for its entire duration, so no other call on the same value
//! can happen concurrently. [`stop_handle`](DataProductConsumerBase::stop_handle)
//! is the adaptation -- obtained *before* calling `run()`, it carries
//! only the shared atomic flag the poll loop checks each iteration, so
//! a caller can `rusty_tokio::spawn` the `run()` future and still call
//! [`ConsumerStopHandle::stop`] from elsewhere, matching issue #91's own
//! framing for this row ("the poll-loop/executor threading model
//! adapted to Rust's async model (`rusty_tokio`) rather than a literal
//! `ThreadPoolExecutor` port").
//!
//! # `event_type` becomes a type parameter
//!
//! The source's `event_type: type[BaseEvent]` class attribute names,
//! at the class level, the event type this consumer's topic carries.
//! Since a `DataProductConsumerBase` subclass declares exactly one
//! `event_type` (unlike a producer's `output_ports`, which can span
//! several event types per instance -- see `producer`'s own module
//! doc), this port makes it a single generic parameter, `E:
//! DomainEvent`, on [`DataProductConsumerBase`] itself rather than a
//! type-erased descriptor: `E::EVENT_NAME` is exactly
//! `event_type.__name__` (SDK-032's `ContractVersionMismatch` check).

use crate::assignor::range_assign;
use crate::registry_client::RegistryClient;
use crate::{ContractVersionMismatch, RegistryError};
use rusty_err::Error;
use rusty_kafka::protocol::consumer_protocol::{
    decode_assignment, decode_subscription, encode_assignment, encode_subscription,
};
use rusty_kafka::protocol::fetch::{FetchPartitionRequest, FetchRequest, FetchTopicRequest};
use rusty_kafka::protocol::find_coordinator::FindCoordinatorRequest;
use rusty_kafka::protocol::heartbeat::HeartbeatRequest;
use rusty_kafka::protocol::join_group::{JoinGroupProtocol, JoinGroupRequest};
use rusty_kafka::protocol::leave_group::LeaveGroupRequest;
use rusty_kafka::protocol::list_offsets::{
    ListOffsetsPartitionRequest, ListOffsetsRequest, ListOffsetsTopicRequest, EARLIEST_TIMESTAMP,
};
use rusty_kafka::protocol::metadata::MetadataRequest;
use rusty_kafka::protocol::offset_commit::{
    OffsetCommitPartitionRequest, OffsetCommitRequest, OffsetCommitTopicRequest,
};
use rusty_kafka::protocol::offset_fetch::{
    OffsetFetchRequest, OffsetFetchTopicRequest, NO_COMMITTED_OFFSET,
};
use rusty_kafka::protocol::sync_group::{SyncGroupAssignment, SyncGroupRequest};
use rusty_kafka::{ClientError, CodecError, KafkaClient};
use rusty_meshed_core::{AvroDecodeError, DomainEvent, PlatformConfig};
use rusty_meshed_observability::LineageTracker;
use rusty_tokio::io::{AsyncRead, AsyncWrite, TcpStream};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Kafka session timeout for this consumer's group membership -- how
/// long the coordinator waits without a `Heartbeat` before considering
/// it dead and triggering a rebalance. No source-config equivalent
/// (the confluent-kafka client's own default, 45s, applies there
/// implicitly); chosen here as a generous multiple of
/// [`FETCH_MAX_WAIT_MS`] so a handful of `process()` calls between
/// heartbeats don't trip it -- see the module doc's "heartbeats and
/// fetches share one connection" note.
const SESSION_TIMEOUT_MS: i32 = 30_000;

/// How long a single `Fetch` call blocks waiting for at least
/// [`FETCH_MIN_BYTES`] before returning empty -- this port's analog of
/// the source's `poll(timeout=1.0)` (SDK-039's own default).
const FETCH_MAX_WAIT_MS: i32 = 1_000;
const FETCH_MIN_BYTES: i32 = 1;
const FETCH_MAX_BYTES: i32 = 1_048_576;

/// The `protocol_type` this crate's consumers always join with --
/// `JoinGroup`'s embedded-payload family (`ConsumerProtocolSubscription`/
/// `ConsumerProtocolAssignment`, see `rusty_kafka::protocol::consumer_protocol`).
const PROTOCOL_TYPE: &str = "consumer";

/// The `protocol_name` this crate's consumers always join with --
/// matches the partition-assignment algorithm [`crate::assignor::range_assign`]
/// actually runs when this member is elected leader.
const PROTOCOL_NAME: &str = "range";

/// Errors from [`DataProductConsumerBase::connect`]/
/// [`DataProductConsumerBase::startup`].
#[derive(Debug, Error)]
pub enum ConsumerStartupError {
    /// A Data Product Registry HTTP call failed.
    #[error("{0}")]
    Registry(#[from] RegistryError),
    /// The resolved output port's JSON was missing a field this method
    /// needs (`topic_name`, `id`, or `data_product_id`).
    #[error("output port response missing '{0}' field")]
    MissingField(&'static str),
    /// The registry's published contract's `schema_ref` didn't contain
    /// `E::EVENT_NAME` (SDK-032).
    #[error("{0}")]
    ContractMismatch(#[from] ContractVersionMismatch),
    /// [`connect`](DataProductConsumerBase::connect)'s
    /// `LineageTracker::new` call failed.
    #[error("lineage tracker initialization failed: {0}")]
    Lineage(String),
    /// A Kafka client call failed while connecting, joining the
    /// consumer group, or resolving starting offsets.
    #[error("{0}")]
    Kafka(#[from] ClientError),
    /// A consumer-group coordination call (`FindCoordinator`/
    /// `JoinGroup`/`SyncGroup`) returned a non-zero Kafka error code.
    /// `{0}` names the call, `{1}` is the broker's error code.
    #[error("broker returned Kafka error code {1} during {0}")]
    BrokerError(&'static str, i16),
    /// The `Metadata` response for the resolved topic was missing or
    /// carried a non-zero topic-level error code (e.g. the topic
    /// doesn't exist yet -- the producer side's own `startup()` must
    /// run first to create it).
    #[error("Metadata returned error code {0} for the topic")]
    TopicMetadata(i16),
    /// A `JoinGroup`/`SyncGroup` embedded consumer-protocol payload
    /// (`metadata`/`assignment`) failed to decode.
    #[error("failed to decode a consumer-protocol payload: {0}")]
    ProtocolCodec(#[from] CodecError),
}

/// Errors from [`DataProductConsumerBase::run`].
#[derive(Debug, Error)]
pub enum ConsumerRunError {
    /// The underlying Kafka client call failed.
    #[error("{0}")]
    Kafka(#[from] ClientError),
    /// A `Heartbeat`/`OffsetCommit` call returned a non-zero Kafka
    /// error code. `{0}` names the call, `{1}` is the broker's error
    /// code -- see the module doc for why this ends the loop rather
    /// than retrying (no automatic rebalance handling).
    #[error("broker returned Kafka error code {1} during {0}")]
    BrokerError(&'static str, i16),
    /// An `OffsetCommit` response had no result for the partition just
    /// committed.
    #[error("no result for the committed partition in the broker's response")]
    MissingPartitionResult,
    /// A fetched record's value bytes failed Avro deserialization --
    /// fatal, matching the source's uncaught deserializer error
    /// propagating straight out of `_poll_loop` (there is no
    /// `try`/`except` there; confluent-kafka's `AvroDeserializer` only
    /// returns `None` for a `None` input, i.e. a tombstone, which this
    /// port already handles separately -- see the module doc).
    #[error("failed to deserialize a fetched record: {0}")]
    Deserialize(#[from] AvroDecodeError),
    /// The caller-supplied `process` callback failed. Ends the loop
    /// without committing that record's offset, matching the source's
    /// uncaught exception propagating out of `_poll_loop` before
    /// `commit()` runs.
    #[error("event processing failed: {0}")]
    Process(String),
}

/// A thread-safe stop switch for [`DataProductConsumerBase::run`],
/// obtained via [`DataProductConsumerBase::stop_handle`] *before*
/// calling `run()` -- see the module doc's "`run()`/`stop()`" section
/// for why this exists instead of a `self._running`-style flag/method
/// pair (SDK-041/042).
#[derive(Clone)]
pub struct ConsumerStopHandle(Arc<AtomicBool>);

impl ConsumerStopHandle {
    /// Signals the owning [`DataProductConsumerBase::run`] loop to
    /// exit after its current iteration (SDK-042's `_running = False`).
    /// Safe to call at any time, including before `run()` starts or
    /// after it has already returned.
    pub fn stop(&self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

/// Abstract base for meshed data product consumers (SDK-029), fully
/// ported: topic/contract resolution (SDK-031..033), consumer-group
/// join/sync and starting-offset resolution
/// ([`startup`](Self::startup), SDK-034..036), event-id dedup
/// (SDK-037/038), and the fetch/dedup/process/commit poll loop
/// ([`run`](Self::run)/[`stop_handle`](Self::stop_handle), SDK-039..042)
/// -- see the module doc for what's structurally different from the
/// source and why.
pub struct DataProductConsumerBase<E, S = TcpStream> {
    product_name: String,
    port_name: String,
    group_id: String,
    registry_client: RegistryClient,
    lineage_tracker: LineageTracker,
    /// Unbounded, in-memory, never evicted -- deduplication is lost on
    /// restart and not shared across processes, matching the source's
    /// own documented limitation (SDK-038) rather than a gap
    /// introduced here.
    seen_event_ids: HashSet<String>,
    client: KafkaClient<S>,
    member_id: String,
    generation_id: i32,
    subscribed_topic: Option<String>,
    assigned_partitions: Vec<i32>,
    next_fetch_offsets: HashMap<i32, i64>,
    running: Arc<AtomicBool>,
    _event_type: PhantomData<E>,
}

impl<E: DomainEvent> DataProductConsumerBase<E, TcpStream> {
    /// Builds a `LineageTracker` and `RegistryClient` from `config`
    /// (SDK-030's DI defaults) and eagerly connects a `KafkaClient` --
    /// see the module doc for why the Kafka connection itself doesn't
    /// wait for `startup()`.
    pub async fn connect(
        product_name: impl Into<String>,
        port_name: impl Into<String>,
        group_id: impl Into<String>,
        config: &PlatformConfig,
    ) -> Result<Self, ConsumerStartupError> {
        let registry_client = RegistryClient::new(config.registry_base_url.clone());
        let lineage_tracker = LineageTracker::new(config.registry_db_path.clone())
            .map_err(|err| ConsumerStartupError::Lineage(err.to_string()))?;
        let client = KafkaClient::connect(
            &config.kafka_bootstrap_servers,
            Some("rusty_meshed_consumer".to_string()),
        )
        .await?;
        Ok(Self::new(
            product_name,
            port_name,
            group_id,
            registry_client,
            lineage_tracker,
            client,
        ))
    }
}

impl<E: DomainEvent, S: AsyncRead + AsyncWrite + Unpin + Send> DataProductConsumerBase<E, S> {
    /// Wraps already-constructed dependencies -- the seam this crate's
    /// own tests use (a fake HTTP server for `registry_client`, an
    /// in-memory `LineageTracker` at a temp path, an in-memory
    /// [`rusty_tokio::io::duplex`] pair for `client`).
    pub fn new(
        product_name: impl Into<String>,
        port_name: impl Into<String>,
        group_id: impl Into<String>,
        registry_client: RegistryClient,
        lineage_tracker: LineageTracker,
        client: KafkaClient<S>,
    ) -> Self {
        DataProductConsumerBase {
            product_name: product_name.into(),
            port_name: port_name.into(),
            group_id: group_id.into(),
            registry_client,
            lineage_tracker,
            seen_event_ids: HashSet::new(),
            client,
            member_id: String::new(),
            generation_id: -1,
            subscribed_topic: None,
            assigned_partitions: Vec::new(),
            next_fetch_offsets: HashMap::new(),
            running: Arc::new(AtomicBool::new(false)),
            _event_type: PhantomData,
        }
    }

    /// This consumer's configured Kafka consumer group ID.
    pub fn group_id(&self) -> &str {
        &self.group_id
    }

    /// Read-only access to this consumer's `LineageTracker` -- used by
    /// [`startup`](Self::startup) itself for SDK-036's post-subscribe
    /// job-run recording, and exposed for callers composing on top of
    /// this base (see `rusty-meshed-domains`).
    pub fn lineage_tracker(&self) -> &LineageTracker {
        &self.lineage_tracker
    }

    /// A cloneable, thread-safe handle to stop a future
    /// [`run`](Self::run) call -- obtain this *before* calling `run()`.
    /// See the module doc's "`run()`/`stop()`" section.
    pub fn stop_handle(&self) -> ConsumerStopHandle {
        ConsumerStopHandle(self.running.clone())
    }

    /// Resolves this consumer's output port and validates its
    /// published contract against `E::EVENT_NAME` (SDK-031..033).
    /// Returns the resolved Kafka topic.
    ///
    /// 1. Resolves the output port via
    ///    `RegistryClient::get_output_port` (SDK-031).
    /// 2. Fetches the port's contract via `RegistryClient::get_contract`;
    ///    skips validation entirely when `None` (no contract published
    ///    yet, SDK-033); when a contract exists and carries a
    ///    `schema_ref` not containing `E::EVENT_NAME`, returns
    ///    [`ConsumerStartupError::ContractMismatch`] (SDK-032).
    pub async fn resolve_output_port(&self) -> Result<String, ConsumerStartupError> {
        let port = self
            .registry_client
            .get_output_port(&self.product_name, &self.port_name)
            .await?;
        let topic = port
            .get("topic_name")
            .and_then(|value| value.as_str())
            .ok_or(ConsumerStartupError::MissingField("topic_name"))?
            .to_string();
        let port_id = port
            .get("id")
            .and_then(|value| value.as_f64())
            .ok_or(ConsumerStartupError::MissingField("id"))? as i64;
        let product_id =
            port.get("data_product_id")
                .and_then(|value| value.as_f64())
                .ok_or(ConsumerStartupError::MissingField("data_product_id"))? as i64;

        if let Some(contract) = self
            .registry_client
            .get_contract(product_id, port_id)
            .await?
        {
            if let Some(schema_ref) = contract.get("schema_ref").and_then(|value| value.as_str()) {
                if !schema_ref.contains(E::EVENT_NAME) {
                    return Err(ContractVersionMismatch::new(E::EVENT_NAME, schema_ref).into());
                }
            }
        }

        Ok(topic)
    }

    /// Returns `false` the first time `event_id` is seen (and records
    /// it), `true` on every subsequent call (SDK-037) -- what
    /// [`run`](Self::run) uses to run `process()` at most once per
    /// unique event ID.
    pub fn is_duplicate(&mut self, event_id: &str) -> bool {
        !self.seen_event_ids.insert(event_id.to_string())
    }

    /// Resolves the topic (SDK-031..033), joins this consumer's group
    /// and computes/receives a partition assignment (SDK-034/035), then
    /// records post-subscribe lineage (SDK-036).
    ///
    /// Joins with an always-empty `member_id` -- every `startup()` call
    /// is a fresh join, never a rejoin with a previously assigned ID.
    /// When this member is elected group leader, computes the whole
    /// group's assignment via [`crate::assignor::range_assign`] (a
    /// `Metadata` call resolves the topic's partition count first);
    /// otherwise sends an empty assignment list and waits for the
    /// leader's via `SyncGroup`, per the Kafka group-membership
    /// protocol. Resolves each assigned partition's starting fetch
    /// offset via `OffsetFetch`, falling back to `ListOffsets` at
    /// [`EARLIEST_TIMESTAMP`] when nothing has been committed yet
    /// (`auto.offset.reset = "earliest"` parity, SDK-034).
    pub async fn startup(&mut self) -> Result<(), ConsumerStartupError> {
        let topic = self.resolve_output_port().await?;
        self.join_and_subscribe(&topic).await?;

        self.lineage_tracker
            .record_job_run(
                &self.consumer_type_name(),
                "meshed",
                &[("kafka".to_string(), topic.clone())],
                &[],
            )
            .map_err(|err| ConsumerStartupError::Lineage(err.to_string()))?;

        self.subscribed_topic = Some(topic);
        Ok(())
    }

    /// `type(self).__name__` has no Rust equivalent for a generic base
    /// -- the source's SDK-036 job-run name is the concrete subclass's
    /// name (e.g. `EmployeeConsumer`), which this composition-based
    /// port has no way to know from inside `DataProductConsumerBase`
    /// itself. `product_name/port_name` is the closest stable
    /// identifier available here, and is what every other lineage call
    /// in this crate family already keys job runs by when no better
    /// name exists.
    fn consumer_type_name(&self) -> String {
        format!("{}/{}", self.product_name, self.port_name)
    }

    async fn join_and_subscribe(&mut self, topic: &str) -> Result<(), ConsumerStartupError> {
        let coordinator = self
            .client
            .find_coordinator(&FindCoordinatorRequest {
                group_id: self.group_id.clone(),
            })
            .await?;
        if coordinator.error_code != 0 {
            return Err(ConsumerStartupError::BrokerError(
                "FindCoordinator",
                coordinator.error_code,
            ));
        }

        let join_response = self
            .client
            .join_group(&JoinGroupRequest {
                group_id: self.group_id.clone(),
                session_timeout_ms: SESSION_TIMEOUT_MS,
                member_id: String::new(),
                protocol_type: PROTOCOL_TYPE.to_string(),
                protocols: vec![JoinGroupProtocol {
                    name: PROTOCOL_NAME.to_string(),
                    metadata: encode_subscription(&[topic.to_string()]),
                }],
            })
            .await?;
        if join_response.error_code != 0 {
            return Err(ConsumerStartupError::BrokerError(
                "JoinGroup",
                join_response.error_code,
            ));
        }
        self.member_id = join_response.member_id.clone();
        self.generation_id = join_response.generation_id;

        let assignments = if join_response.is_leader() {
            self.compute_leader_assignment(topic, &join_response.members)
                .await?
        } else {
            Vec::new()
        };

        let sync_response = self
            .client
            .sync_group(&SyncGroupRequest {
                group_id: self.group_id.clone(),
                generation_id: self.generation_id,
                member_id: self.member_id.clone(),
                assignments,
            })
            .await?;
        if sync_response.error_code != 0 {
            return Err(ConsumerStartupError::BrokerError(
                "SyncGroup",
                sync_response.error_code,
            ));
        }

        let my_assignment = decode_assignment(&sync_response.assignment)?;
        let mut partitions: Vec<i32> = my_assignment
            .into_iter()
            .find(|(assigned_topic, _)| assigned_topic == topic)
            .map(|(_, partitions)| partitions)
            .unwrap_or_default();
        partitions.sort_unstable();
        self.assigned_partitions = partitions;

        for &partition_index in &self.assigned_partitions.clone() {
            let start_offset = self.resolve_starting_offset(topic, partition_index).await?;
            self.next_fetch_offsets
                .insert(partition_index, start_offset);
        }

        Ok(())
    }

    /// Computes the whole group's partition assignment (this member is
    /// the elected leader): a `Metadata` call resolves `topic`'s
    /// partition count, every member's declared subscription is
    /// decoded from `members`, and [`range_assign`] splits partitions
    /// among whichever members actually subscribed to each topic.
    async fn compute_leader_assignment(
        &mut self,
        topic: &str,
        members: &[rusty_kafka::protocol::join_group::JoinGroupMember],
    ) -> Result<Vec<SyncGroupAssignment>, ConsumerStartupError> {
        let metadata_response = self
            .client
            .metadata(&MetadataRequest {
                topics: Some(vec![topic.to_string()]),
            })
            .await?;
        let topic_metadata = metadata_response
            .topics
            .first()
            .ok_or(ConsumerStartupError::TopicMetadata(-1))?;
        if topic_metadata.error_code != 0 {
            return Err(ConsumerStartupError::TopicMetadata(
                topic_metadata.error_code,
            ));
        }
        let mut partition_counts = BTreeMap::new();
        partition_counts.insert(topic.to_string(), topic_metadata.partitions.len() as i32);

        let mut subscriptions = Vec::with_capacity(members.len());
        for member in members {
            let topics = decode_subscription(&member.metadata)?;
            subscriptions.push((member.member_id.clone(), topics));
        }

        let assignment_by_member = range_assign(&subscriptions, &partition_counts);
        Ok(assignment_by_member
            .into_iter()
            .map(|(member_id, assignment)| SyncGroupAssignment {
                member_id,
                assignment: encode_assignment(&assignment),
            })
            .collect())
    }

    /// The offset to start fetching `partition_index` from: the
    /// group's previously committed offset, or (nothing committed yet)
    /// the topic's earliest available offset -- `auto.offset.reset =
    /// "earliest"` parity (SDK-034).
    async fn resolve_starting_offset(
        &mut self,
        topic: &str,
        partition_index: i32,
    ) -> Result<i64, ConsumerStartupError> {
        let offset_fetch_response = self
            .client
            .offset_fetch(&OffsetFetchRequest {
                group_id: self.group_id.clone(),
                topics: vec![OffsetFetchTopicRequest {
                    name: topic.to_string(),
                    partitions: vec![partition_index],
                }],
            })
            .await?;
        let committed = offset_fetch_response
            .topics
            .first()
            .and_then(|t| t.partitions.first())
            .map(|p| p.committed_offset)
            .unwrap_or(NO_COMMITTED_OFFSET);
        if committed != NO_COMMITTED_OFFSET {
            return Ok(committed);
        }

        let list_offsets_response = self
            .client
            .list_offsets(&ListOffsetsRequest {
                replica_id: -1,
                topics: vec![ListOffsetsTopicRequest {
                    name: topic.to_string(),
                    partitions: vec![ListOffsetsPartitionRequest {
                        partition_index,
                        timestamp: EARLIEST_TIMESTAMP,
                    }],
                }],
            })
            .await?;
        Ok(list_offsets_response
            .topics
            .first()
            .and_then(|t| t.partitions.first())
            .map(|p| p.offset)
            .unwrap_or(0))
    }

    /// Runs the fetch/dedup/process/commit poll loop until
    /// [`ConsumerStopHandle::stop`] is called (SDK-039..042). A no-op
    /// returning `Ok(())` immediately if [`startup`](Self::startup)
    /// hasn't run yet -- matching the source's own `if self._consumer
    /// is None: break`.
    ///
    /// Each iteration: sends one `Heartbeat`, then one `Fetch` covering
    /// every assigned partition (see the module doc for why they share
    /// one connection). Every returned record, in order: skipped if its
    /// value is `None` (a tombstone -- matches the source's deserializer
    /// short-circuiting on a `None` input, SDK-039's `if event is None`);
    /// deserialized via `E::deserialize`, a hard failure ending the loop
    /// (the source's own uncaught deserializer error would too); skipped
    /// if [`is_duplicate`](Self::is_duplicate) says so; otherwise `process`
    /// is awaited and, only once it succeeds, that record's offset is
    /// committed via `OffsetCommit` before moving to the next one
    /// (SDK-040).
    ///
    /// On exit (the stop handle was signaled), sends a best-effort
    /// `LeaveGroup` -- this port's `consumer.close()` equivalent
    /// (SDK-042); its result is intentionally discarded; a failed leave
    /// just means the coordinator evicts this member after
    /// `session.timeout.ms` instead of immediately.
    pub async fn run<F, Fut>(&mut self, mut process: F) -> Result<(), ConsumerRunError>
    where
        F: FnMut(E) -> Fut,
        Fut: Future<Output = Result<(), String>>,
    {
        let Some(topic) = self.subscribed_topic.clone() else {
            return Ok(());
        };

        self.running.store(true, Ordering::Relaxed);
        while self.running.load(Ordering::Relaxed) {
            self.send_heartbeat().await?;
            let response = self.fetch_batch(&topic).await?;

            for topic_response in &response.topics {
                for partition_response in &topic_response.partitions {
                    if partition_response.error_code != 0 {
                        continue;
                    }
                    let partition_index = partition_response.partition_index;
                    let base_offset = self.next_fetch_offsets[&partition_index];

                    for (index, record) in partition_response.records.iter().enumerate() {
                        let Some(value) = &record.value else {
                            continue;
                        };
                        let event = E::deserialize(value)?;
                        if self.is_duplicate(&event.base().event_id) {
                            continue;
                        }

                        process(event).await.map_err(ConsumerRunError::Process)?;

                        let next_offset = base_offset + index as i64 + 1;
                        self.commit_offset(&topic, partition_index, next_offset)
                            .await?;
                    }
                }
            }
        }

        let _ = self
            .client
            .leave_group(&LeaveGroupRequest {
                group_id: self.group_id.clone(),
                member_id: self.member_id.clone(),
            })
            .await;
        Ok(())
    }

    async fn send_heartbeat(&mut self) -> Result<(), ConsumerRunError> {
        let response = self
            .client
            .heartbeat(&HeartbeatRequest {
                group_id: self.group_id.clone(),
                generation_id: self.generation_id,
                member_id: self.member_id.clone(),
            })
            .await?;
        if response.error_code != 0 {
            return Err(ConsumerRunError::BrokerError(
                "Heartbeat",
                response.error_code,
            ));
        }
        Ok(())
    }

    async fn fetch_batch(
        &mut self,
        topic: &str,
    ) -> Result<rusty_kafka::protocol::fetch::FetchResponse, ConsumerRunError> {
        let partitions: Vec<FetchPartitionRequest> = self
            .assigned_partitions
            .iter()
            .map(|&partition_index| FetchPartitionRequest {
                partition_index,
                fetch_offset: self.next_fetch_offsets[&partition_index],
                partition_max_bytes: FETCH_MAX_BYTES,
            })
            .collect();
        let request = FetchRequest {
            max_wait_ms: FETCH_MAX_WAIT_MS,
            min_bytes: FETCH_MIN_BYTES,
            max_bytes: FETCH_MAX_BYTES,
            topics: vec![FetchTopicRequest {
                name: topic.to_string(),
                partitions,
            }],
            ..Default::default()
        };
        Ok(self.client.fetch(&request).await?)
    }

    async fn commit_offset(
        &mut self,
        topic: &str,
        partition_index: i32,
        next_offset: i64,
    ) -> Result<(), ConsumerRunError> {
        let response = self
            .client
            .offset_commit(&OffsetCommitRequest {
                group_id: self.group_id.clone(),
                group_generation_id: self.generation_id,
                member_id: self.member_id.clone(),
                retention_time_ms: -1,
                topics: vec![OffsetCommitTopicRequest {
                    name: topic.to_string(),
                    partitions: vec![OffsetCommitPartitionRequest {
                        partition_index,
                        committed_offset: next_offset,
                        committed_metadata: None,
                    }],
                }],
            })
            .await?;
        let result = response
            .topics
            .first()
            .and_then(|t| t.partitions.first())
            .ok_or(ConsumerRunError::MissingPartitionResult)?;
        if result.error_code != 0 {
            return Err(ConsumerRunError::BrokerError(
                "OffsetCommit",
                result.error_code,
            ));
        }
        self.next_fetch_offsets.insert(partition_index, next_offset);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_http::async_tokio::AsyncTransport;
    use rusty_http::head::ResponseHead;
    use rusty_http::{HeaderMap, StatusCode, Version};
    use rusty_kafka::protocol::api_key;
    use rusty_kafka::protocol::fetch::{FetchPartitionResponse, FetchResponse, FetchTopicResponse};
    use rusty_kafka::protocol::join_group::{JoinGroupMember, JoinGroupResponse};
    use rusty_kafka::protocol::leave_group::LeaveGroupResponse;
    use rusty_kafka::protocol::metadata::{MetadataResponse, PartitionMetadata, TopicMetadata};
    use rusty_kafka::protocol::offset_commit::{
        OffsetCommitPartitionResponse, OffsetCommitResponse, OffsetCommitTopicResponse,
    };
    use rusty_kafka::protocol::offset_fetch::{
        OffsetFetchPartitionResponse, OffsetFetchResponse, OffsetFetchTopicResponse,
    };
    use rusty_kafka::protocol::sync_group::SyncGroupResponse;
    use rusty_kafka::record_batch::Record;
    use rusty_kafka::testing::{recv_request, send_response};
    use rusty_meshed_core::{AvroDecodeError, BaseEvent};
    use rusty_tokio::io::{duplex, DuplexStream};
    use rusty_wire::Writer;

    struct TestEvent {
        base: BaseEvent,
    }

    impl DomainEvent for TestEvent {
        const EVENT_NAME: &'static str = "TestEvent";

        fn base(&self) -> &BaseEvent {
            &self.base
        }

        fn avro_schema() -> String {
            BaseEvent::avro_record_schema("TestEvent", "test", rusty_json::json!([]))
        }

        fn serialize(&self) -> Vec<u8> {
            self.base.serialize()
        }

        fn deserialize(bytes: &[u8]) -> Result<Self, AvroDecodeError> {
            Ok(TestEvent {
                base: BaseEvent::deserialize(bytes)?,
            })
        }

        fn to_json(&self) -> rusty_json::Value {
            rusty_json::Value::Object(self.base.to_json_fields())
        }
    }

    fn temp_db_path(name: &str) -> String {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "rusty_meshed_consumer_test_{name}_{}.db",
            rusty_uuid::Uuid::new_v4()
        ));
        path.to_str().unwrap().to_string()
    }

    struct CapturedHttpRequest {
        target: String,
    }

    /// A local copy of the same fake-HTTP-server helper `producer`'s
    /// own tests use -- see that module's test doc comment for why
    /// it's duplicated rather than shared.
    fn start_fake_http_server(
        responses: Vec<(u16, &'static str)>,
    ) -> (String, rusty_tokio::JoinHandle<Vec<CapturedHttpRequest>>) {
        let listener = rusty_tokio::io::TcpListener::bind("127.0.0.1:0".parse().unwrap())
            .expect("failed to bind");
        let addr = listener.local_addr().expect("failed to read local_addr");
        let url = format!("http://{addr}");

        let handle = rusty_tokio::spawn(async move {
            let mut captured = Vec::new();
            for (status, response_body) in responses {
                let (stream, _peer) = listener.accept().await.expect("failed to accept");
                let mut transport = AsyncTransport::new(stream);
                let head = transport
                    .read_request_head(8192)
                    .await
                    .expect("failed to read request head");
                let framing = rusty_http::body::request_framing(&head.headers)
                    .expect("failed to determine framing");
                let _body_bytes = transport
                    .read_body(framing)
                    .await
                    .expect("failed to read request body");

                let mut headers = HeaderMap::new();
                headers
                    .insert("Content-Length", &response_body.len().to_string())
                    .unwrap();
                headers.insert("Content-Type", "application/json").unwrap();
                let response_head = ResponseHead {
                    status: StatusCode::from_u16(status),
                    reason: String::new(),
                    version: Version::Http11,
                    headers,
                };
                transport
                    .write_response_head(&response_head)
                    .await
                    .expect("failed to write response head");
                transport
                    .write_body(response_body.as_bytes())
                    .await
                    .expect("failed to write response body");

                captured.push(CapturedHttpRequest {
                    target: head.target.clone(),
                });
            }
            captured
        });

        (url, handle)
    }

    fn consumer_with(
        registry_client: RegistryClient,
        client: KafkaClient<DuplexStream>,
    ) -> DataProductConsumerBase<TestEvent, DuplexStream> {
        let db_path = temp_db_path("resolve");
        let lineage_tracker = LineageTracker::new(&db_path).unwrap();
        DataProductConsumerBase::new(
            "personnel-lifecycle",
            "assignments",
            "readiness-reporting-personnel-consumer",
            registry_client,
            lineage_tracker,
            client,
        )
    }

    fn unused_kafka_client() -> (KafkaClient<DuplexStream>, DuplexStream) {
        let (client_io, peer) = duplex(4096);
        (KafkaClient::new(client_io, None), peer)
    }

    #[rusty_tokio::test]
    async fn resolve_output_port_returns_the_topic_with_no_contract_published() {
        let (url, server) = start_fake_http_server(vec![
            (200, r#"[{"id": 1, "name": "personnel-lifecycle"}]"#),
            (
                200,
                r#"[{"id": 10, "data_product_id": 1, "description": "assignments", "topic_name": "manpower.personnel-lifecycle.assignments"}]"#,
            ),
            (404, "{}"),
        ]);
        let (client, _peer) = unused_kafka_client();
        let consumer = consumer_with(RegistryClient::new(&url), client);

        let topic = consumer.resolve_output_port().await.unwrap();
        assert_eq!(topic, "manpower.personnel-lifecycle.assignments");

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[2].target.contains("/contract"));
    }

    #[rusty_tokio::test]
    async fn resolve_output_port_passes_when_the_contract_schema_ref_contains_the_event_name() {
        let (url, _server) = start_fake_http_server(vec![
            (200, r#"[{"id": 1, "name": "personnel-lifecycle"}]"#),
            (
                200,
                r#"[{"id": 10, "data_product_id": 1, "description": "assignments", "topic_name": "manpower.personnel-lifecycle.assignments"}]"#,
            ),
            (200, r#"{"schema_ref": "TestEvent-value-1"}"#),
        ]);
        let (client, _peer) = unused_kafka_client();
        let consumer = consumer_with(RegistryClient::new(&url), client);

        let topic = consumer.resolve_output_port().await.unwrap();
        assert_eq!(topic, "manpower.personnel-lifecycle.assignments");
    }

    #[rusty_tokio::test]
    async fn resolve_output_port_fails_when_the_contract_schema_ref_omits_the_event_name() {
        let (url, _server) = start_fake_http_server(vec![
            (200, r#"[{"id": 1, "name": "personnel-lifecycle"}]"#),
            (
                200,
                r#"[{"id": 10, "data_product_id": 1, "description": "assignments", "topic_name": "manpower.personnel-lifecycle.assignments"}]"#,
            ),
            (200, r#"{"schema_ref": "SomeOtherEvent-value-1"}"#),
        ]);
        let (client, _peer) = unused_kafka_client();
        let consumer = consumer_with(RegistryClient::new(&url), client);

        let err = consumer.resolve_output_port().await.unwrap_err();
        assert!(matches!(err, ConsumerStartupError::ContractMismatch(_)));
    }

    #[test]
    fn is_duplicate_returns_false_once_then_true_on_repeats() {
        let (client, _peer) = unused_kafka_client();
        let mut consumer = consumer_with(RegistryClient::new("http://unused.invalid"), client);
        assert!(!consumer.is_duplicate("evt-1"));
        assert!(consumer.is_duplicate("evt-1"));
        assert!(!consumer.is_duplicate("evt-2"));
    }

    /// Drives one full `join_and_subscribe` round trip as the sole
    /// (and therefore leader) group member: `FindCoordinator` ->
    /// `JoinGroup` -> `Metadata` (3 partitions) -> `SyncGroup` (all 3
    /// assigned back) -> `OffsetFetch` x3 (nothing committed) ->
    /// `ListOffsets` x3 (earliest = 0).
    async fn respond_to_join_and_subscribe(peer: &mut DuplexStream, topic: &str) {
        let (header, _body) = recv_request(peer).await.unwrap();
        assert_eq!(header.api_key, api_key::FIND_COORDINATOR);
        let response = rusty_kafka::protocol::find_coordinator::FindCoordinatorResponse {
            error_code: 0,
            node_id: 1,
            host: "localhost".to_string(),
            port: 9092,
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        send_response(peer, header.correlation_id, writer.as_slice())
            .await
            .unwrap();

        let (header, _body) = recv_request(peer).await.unwrap();
        assert_eq!(header.api_key, api_key::JOIN_GROUP);
        let response = JoinGroupResponse {
            error_code: 0,
            generation_id: 1,
            group_protocol: "range".to_string(),
            leader_id: "consumer-1".to_string(),
            member_id: "consumer-1".to_string(),
            members: vec![JoinGroupMember {
                member_id: "consumer-1".to_string(),
                metadata: encode_subscription(&[topic.to_string()]),
            }],
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        send_response(peer, header.correlation_id, writer.as_slice())
            .await
            .unwrap();

        let (header, _body) = recv_request(peer).await.unwrap();
        assert_eq!(header.api_key, api_key::METADATA);
        let response = MetadataResponse {
            brokers: vec![],
            topics: vec![TopicMetadata {
                error_code: 0,
                name: topic.to_string(),
                partitions: (0..3)
                    .map(|partition_index| PartitionMetadata {
                        error_code: 0,
                        partition_index,
                        leader_id: 1,
                        replica_nodes: vec![1],
                        isr_nodes: vec![1],
                    })
                    .collect(),
            }],
        };
        let mut writer = Writer::new();
        write_metadata_response(&mut writer, &response);
        send_response(peer, header.correlation_id, writer.as_slice())
            .await
            .unwrap();

        let (header, _body) = recv_request(peer).await.unwrap();
        assert_eq!(header.api_key, api_key::SYNC_GROUP);
        let response = SyncGroupResponse {
            error_code: 0,
            assignment: encode_assignment(&[(topic.to_string(), vec![0, 1, 2])]),
        };
        let mut writer = Writer::new();
        response.encode(&mut writer);
        send_response(peer, header.correlation_id, writer.as_slice())
            .await
            .unwrap();

        for _ in 0..3 {
            let (header, _body) = recv_request(peer).await.unwrap();
            assert_eq!(header.api_key, api_key::OFFSET_FETCH);
            let response = OffsetFetchResponse {
                topics: vec![OffsetFetchTopicResponse {
                    name: topic.to_string(),
                    partitions: vec![OffsetFetchPartitionResponse {
                        partition_index: 0,
                        committed_offset: NO_COMMITTED_OFFSET,
                        metadata: None,
                        error_code: 0,
                    }],
                }],
            };
            let mut writer = Writer::new();
            response.encode(&mut writer);
            send_response(peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();

            let (header, _body) = recv_request(peer).await.unwrap();
            assert_eq!(header.api_key, api_key::LIST_OFFSETS);
            let response = rusty_kafka::protocol::list_offsets::ListOffsetsResponse {
                topics: vec![
                    rusty_kafka::protocol::list_offsets::ListOffsetsTopicResponse {
                        name: topic.to_string(),
                        partitions: vec![
                            rusty_kafka::protocol::list_offsets::ListOffsetsPartitionResponse {
                                partition_index: 0,
                                error_code: 0,
                                timestamp: -1,
                                offset: 0,
                            },
                        ],
                    },
                ],
            };
            let mut writer = Writer::new();
            response.encode(&mut writer);
            send_response(peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();
        }
    }

    /// `MetadataResponse` has no `encode` (this crate is client-only,
    /// see `rusty_kafka`'s own module doc) -- hand-encodes the v0 wire
    /// shape the fake broker needs to send back, symmetric with
    /// `MetadataResponse::decode`. `rusty_kafka::wire` itself is a
    /// private module, so this writes the same `INT16`/`INT32`/
    /// `STRING` primitives directly through `rusty_wire::Writer`.
    fn write_metadata_response(writer: &mut Writer, response: &MetadataResponse) {
        fn write_i16(writer: &mut Writer, v: i16) {
            writer.write_u16_be(v as u16);
        }
        fn write_i32(writer: &mut Writer, v: i32) {
            writer.write_u32_be(v as u32);
        }
        fn write_string(writer: &mut Writer, v: &str) {
            write_i16(writer, v.len() as i16);
            writer.write_bytes(v.as_bytes());
        }

        write_i32(writer, response.brokers.len() as i32);
        for broker in &response.brokers {
            write_i32(writer, broker.node_id);
            write_string(writer, &broker.host);
            write_i32(writer, broker.port);
        }
        write_i32(writer, response.topics.len() as i32);
        for topic in &response.topics {
            write_i16(writer, topic.error_code);
            write_string(writer, &topic.name);
            write_i32(writer, topic.partitions.len() as i32);
            for partition in &topic.partitions {
                write_i16(writer, partition.error_code);
                write_i32(writer, partition.partition_index);
                write_i32(writer, partition.leader_id);
                write_i32(writer, partition.replica_nodes.len() as i32);
                for node in &partition.replica_nodes {
                    write_i32(writer, *node);
                }
                write_i32(writer, partition.isr_nodes.len() as i32);
                for node in &partition.isr_nodes {
                    write_i32(writer, *node);
                }
            }
        }
    }

    #[rusty_tokio::test]
    async fn startup_joins_as_leader_and_resolves_earliest_offsets() {
        let topic = "manpower.personnel-lifecycle.assignments";
        let (url, _server) = start_fake_http_server(vec![
            (200, r#"[{"id": 1, "name": "personnel-lifecycle"}]"#),
            (
                200,
                r#"[{"id": 10, "data_product_id": 1, "description": "assignments", "topic_name": "manpower.personnel-lifecycle.assignments"}]"#,
            ),
            (404, "{}"),
        ]);
        let (client_io, mut peer) = duplex(8192);
        let client = KafkaClient::new(client_io, None);
        let mut consumer = consumer_with(RegistryClient::new(&url), client);

        let server = rusty_tokio::spawn(async move {
            respond_to_join_and_subscribe(&mut peer, topic).await;
        });

        consumer.startup().await.unwrap();
        server.await.unwrap();

        assert_eq!(consumer.assigned_partitions, vec![0, 1, 2]);
        assert_eq!(consumer.next_fetch_offsets[&0], 0);
        assert_eq!(consumer.subscribed_topic.as_deref(), Some(topic));
    }

    #[rusty_tokio::test]
    async fn run_processes_a_fetched_record_then_commits_and_stops() {
        let (client_io, mut peer) = duplex(8192);
        let client = KafkaClient::new(client_io, None);
        let mut consumer = consumer_with(RegistryClient::new("http://unused.invalid"), client);
        consumer.member_id = "consumer-1".to_string();
        consumer.generation_id = 1;
        consumer.subscribed_topic = Some("t".to_string());
        consumer.assigned_partitions = vec![0];
        consumer.next_fetch_offsets.insert(0, 5);

        let stop_handle = consumer.stop_handle();

        let event = TestEvent {
            base: BaseEvent::new("req-1"),
        };
        let event_id = event.base.event_id.clone();
        let value = event.serialize();

        let server = rusty_tokio::spawn(async move {
            let (header, _body) = recv_request(&mut peer).await.unwrap();
            assert_eq!(header.api_key, api_key::HEARTBEAT);
            let response = rusty_kafka::protocol::heartbeat::HeartbeatResponse { error_code: 0 };
            let mut writer = Writer::new();
            response.encode(&mut writer);
            send_response(&mut peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();

            let (header, _body) = recv_request(&mut peer).await.unwrap();
            assert_eq!(header.api_key, api_key::FETCH);
            let response = FetchResponse {
                throttle_time_ms: 0,
                topics: vec![FetchTopicResponse {
                    name: "t".to_string(),
                    partitions: vec![FetchPartitionResponse {
                        partition_index: 0,
                        error_code: 0,
                        high_watermark: 6,
                        last_stable_offset: 6,
                        aborted_transactions: vec![],
                        records: vec![Record {
                            key: None,
                            value: Some(value),
                            headers: vec![],
                        }],
                    }],
                }],
            };
            let mut writer = Writer::new();
            response.encode(&mut writer, 1_735_689_600_000);
            send_response(&mut peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();

            let (header, body) = recv_request(&mut peer).await.unwrap();
            assert_eq!(header.api_key, api_key::OFFSET_COMMIT);
            let mut reader = rusty_wire::Reader::new(&body);
            let decoded = OffsetCommitRequest::decode(&mut reader).unwrap();
            assert_eq!(decoded.topics[0].partitions[0].committed_offset, 6);
            let response = OffsetCommitResponse {
                topics: vec![OffsetCommitTopicResponse {
                    name: "t".to_string(),
                    partitions: vec![OffsetCommitPartitionResponse {
                        partition_index: 0,
                        error_code: 0,
                    }],
                }],
            };
            let mut writer = Writer::new();
            response.encode(&mut writer);

            // Stop *before* sending the response the loop's next
            // `running` check races against: this is a multi-threaded
            // runtime, so "call stop() after the response is sent" is
            // a genuine data race against the client task -- it could
            // observe the still-true flag and send a second Heartbeat
            // before this task's `stop()` call is even scheduled.
            // Calling it first means the client can only see `running
            // == false` once it reads this response.
            stop_handle.stop();
            send_response(&mut peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();

            let (header, _body) = recv_request(&mut peer).await.unwrap();
            assert_eq!(header.api_key, api_key::LEAVE_GROUP);
            let response = LeaveGroupResponse { error_code: 0 };
            let mut writer = Writer::new();
            response.encode(&mut writer);
            send_response(&mut peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();
        });

        let mut processed = Vec::new();
        consumer
            .run(|event: TestEvent| {
                processed.push(event.base.event_id.clone());
                async { Ok(()) }
            })
            .await
            .unwrap();
        server.await.unwrap();

        assert_eq!(processed, vec![event_id]);
        assert_eq!(consumer.next_fetch_offsets[&0], 6);
    }

    #[rusty_tokio::test]
    async fn run_is_a_noop_when_startup_has_not_run() {
        let (client, _peer) = unused_kafka_client();
        let mut consumer = consumer_with(RegistryClient::new("http://unused.invalid"), client);

        consumer.run(|_: TestEvent| async { Ok(()) }).await.unwrap();
    }

    #[rusty_tokio::test]
    async fn run_skips_a_duplicate_record_without_reprocessing_it() {
        let (client_io, mut peer) = duplex(8192);
        let client = KafkaClient::new(client_io, None);
        let mut consumer = consumer_with(RegistryClient::new("http://unused.invalid"), client);
        consumer.member_id = "consumer-1".to_string();
        consumer.generation_id = 1;
        consumer.subscribed_topic = Some("t".to_string());
        consumer.assigned_partitions = vec![0];
        consumer.next_fetch_offsets.insert(0, 0);

        let event = TestEvent {
            base: BaseEvent::new("req-1"),
        };
        let event_id = event.base.event_id.clone();
        consumer.is_duplicate(&event_id); // seed as already seen
        let value = event.serialize();

        let stop_handle = consumer.stop_handle();
        let server = rusty_tokio::spawn(async move {
            let (header, _body) = recv_request(&mut peer).await.unwrap();
            assert_eq!(header.api_key, api_key::HEARTBEAT);
            let response = rusty_kafka::protocol::heartbeat::HeartbeatResponse { error_code: 0 };
            let mut writer = Writer::new();
            response.encode(&mut writer);
            send_response(&mut peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();

            let (header, _body) = recv_request(&mut peer).await.unwrap();
            assert_eq!(header.api_key, api_key::FETCH);
            let response = FetchResponse {
                throttle_time_ms: 0,
                topics: vec![FetchTopicResponse {
                    name: "t".to_string(),
                    partitions: vec![FetchPartitionResponse {
                        partition_index: 0,
                        error_code: 0,
                        high_watermark: 1,
                        last_stable_offset: 1,
                        aborted_transactions: vec![],
                        records: vec![Record {
                            key: None,
                            value: Some(value),
                            headers: vec![],
                        }],
                    }],
                }],
            };
            let mut writer = Writer::new();
            response.encode(&mut writer, 1_735_689_600_000);

            // Stop before sending, not after -- see the sibling test's
            // own comment on why calling it after would race the
            // client's next `running` check on this multi-threaded
            // runtime. No OffsetCommit expected either way -- the
            // record was a duplicate.
            stop_handle.stop();
            send_response(&mut peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();

            let (header, _body) = recv_request(&mut peer).await.unwrap();
            assert_eq!(header.api_key, api_key::LEAVE_GROUP);
            let response = LeaveGroupResponse { error_code: 0 };
            let mut writer = Writer::new();
            response.encode(&mut writer);
            send_response(&mut peer, header.correlation_id, writer.as_slice())
                .await
                .unwrap();
        });

        let mut processed = 0;
        consumer
            .run(|_: TestEvent| {
                processed += 1;
                async { Ok(()) }
            })
            .await
            .unwrap();
        server.await.unwrap();

        assert_eq!(processed, 0);
        assert_eq!(consumer.next_fetch_offsets[&0], 0);
    }
}

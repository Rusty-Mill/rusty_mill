//! A Redis-backed [`Store`], for running several replicas behind a load
//! balancer.

use std::time::Duration;

use futures_util::StreamExt;
use redis::{aio::ConnectionManager, AsyncCommands, Client};

use crate::{
    server::store::{
        message_url, state_url, Notification, NotificationStream, SessionRecord, Store, StoreResult,
    },
    types::{Error, Event, Message, Run, RunId, Session, SessionId},
};

/// Default time-to-live applied to run, event and session keys.
pub const DEFAULT_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Default prefix for every key and channel this store owns.
pub const DEFAULT_KEY_PREFIX: &str = "acp";

/// How a [`RedisStore`] names and expires its keys.
#[derive(Debug, Clone)]
pub struct RedisStoreConfig {
    /// Prefix for every key and channel. Lets several deployments share one
    /// Redis instance.
    pub key_prefix: String,
    /// How long runs, event logs and sessions live after their last write.
    ///
    /// The ACP high-availability guide calls for automatic expiration; this is
    /// how it is applied. `None` keeps keys forever, which needs external
    /// cleanup.
    pub ttl: Option<Duration>,
}

impl Default for RedisStoreConfig {
    fn default() -> Self {
        Self { key_prefix: DEFAULT_KEY_PREFIX.to_string(), ttl: Some(DEFAULT_TTL) }
    }
}

/// Session metadata held alongside the message list.
///
/// History URLs are *derived* from message indices at read time rather than
/// stored, which is what lets an append be a single atomic `RPUSH`: the URL for
/// message `i` is a pure function of `i`.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct SessionMeta {
    id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    /// History entries supplied by the client, hosted elsewhere. These come
    /// before anything this deployment stores.
    #[serde(default)]
    prefix_history: Vec<String>,
    /// Base URL recorded on first append, so reads can rebuild history URLs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base_url: Option<String>,
}

/// Keeps runs and sessions in Redis, using pub/sub for notifications.
///
/// Point every replica at the same Redis instance and they share one view of
/// every run: any replica can serve any request, and control signals reach
/// whichever replica is executing the agent. This is the backend the ACP
/// [high-availability guide][ha] names first.
///
/// ```no_run
/// # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
/// use rusty_acp::server::store::RedisStore;
///
/// let store = RedisStore::connect("redis://127.0.0.1/").await?;
/// # Ok(())
/// # }
/// ```
///
/// [ha]: https://agentcommunicationprotocol.dev/how-to/high-availability
#[derive(Clone)]
pub struct RedisStore {
    client: Client,
    connection: ConnectionManager,
    config: RedisStoreConfig,
}

impl std::fmt::Debug for RedisStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisStore").field("config", &self.config).finish_non_exhaustive()
    }
}

impl RedisStore {
    /// Connect with the default configuration.
    pub async fn connect(url: &str) -> StoreResult<Self> {
        Self::connect_with(url, RedisStoreConfig::default()).await
    }

    /// Connect with an explicit configuration.
    pub async fn connect_with(url: &str, config: RedisStoreConfig) -> StoreResult<Self> {
        let client = Client::open(url).map_err(|err| redis_error("open Redis client", err))?;
        let connection = ConnectionManager::new(client.clone())
            .await
            .map_err(|err| redis_error("connect to Redis", err))?;
        Ok(Self { client, connection, config })
    }

    /// The configuration in use.
    pub fn config(&self) -> &RedisStoreConfig {
        &self.config
    }

    fn run_key(&self, run_id: RunId) -> String {
        format!("{}:run:{run_id}", self.config.key_prefix)
    }

    fn events_key(&self, run_id: RunId) -> String {
        format!("{}:events:{run_id}", self.config.key_prefix)
    }

    fn channel(&self, run_id: RunId) -> String {
        format!("{}:chan:{run_id}", self.config.key_prefix)
    }

    fn session_meta_key(&self, session_id: SessionId) -> String {
        format!("{}:session:{session_id}:meta", self.config.key_prefix)
    }

    fn session_messages_key(&self, session_id: SessionId) -> String {
        format!("{}:session:{session_id}:messages", self.config.key_prefix)
    }

    fn session_state_key(&self, session_id: SessionId) -> String {
        format!("{}:session:{session_id}:state", self.config.key_prefix)
    }

    fn ttl_seconds(&self) -> Option<i64> {
        self.config.ttl.map(|ttl| ttl.as_secs().max(1) as i64)
    }

    /// Refresh a key's expiry, if a TTL is configured.
    async fn touch(&self, key: &str) -> StoreResult<()> {
        if let Some(seconds) = self.ttl_seconds() {
            let mut connection = self.connection.clone();
            let _: () = connection
                .expire(key, seconds)
                .await
                .map_err(|err| redis_error("set key expiry", err))?;
        }
        Ok(())
    }
}

fn redis_error(action: &str, err: redis::RedisError) -> Error {
    Error::server_error(format!("failed to {action}: {err}"))
}

fn encode<T: serde::Serialize>(value: &T) -> StoreResult<String> {
    serde_json::to_string(value)
        .map_err(|err| Error::server_error(format!("failed to encode for Redis: {err}")))
}

fn decode<T: serde::de::DeserializeOwned>(raw: &str) -> StoreResult<T> {
    serde_json::from_str(raw)
        .map_err(|err| Error::server_error(format!("failed to decode from Redis: {err}")))
}

#[async_trait::async_trait]
impl Store for RedisStore {
    async fn put_run(&self, run: &Run) -> StoreResult<()> {
        let key = self.run_key(run.run_id);
        let mut connection = self.connection.clone();
        let payload = encode(run)?;
        match self.ttl_seconds() {
            Some(seconds) => {
                let _: () = connection
                    .set_ex(&key, payload, seconds as u64)
                    .await
                    .map_err(|err| redis_error("write run", err))?;
            }
            None => {
                let _: () = connection
                    .set(&key, payload)
                    .await
                    .map_err(|err| redis_error("write run", err))?;
            }
        }
        Ok(())
    }

    async fn get_run(&self, run_id: RunId) -> StoreResult<Option<Run>> {
        let mut connection = self.connection.clone();
        let raw: Option<String> = connection
            .get(self.run_key(run_id))
            .await
            .map_err(|err| redis_error("read run", err))?;
        raw.as_deref().map(decode).transpose()
    }

    async fn append_event(&self, run_id: RunId, event: &Event) -> StoreResult<()> {
        let key = self.events_key(run_id);
        let mut connection = self.connection.clone();
        // RPUSH is atomic, so concurrent appends cannot interleave or be lost.
        let _: () = connection
            .rpush(&key, encode(event)?)
            .await
            .map_err(|err| redis_error("append event", err))?;
        self.touch(&key).await
    }

    async fn events(&self, run_id: RunId) -> StoreResult<Vec<Event>> {
        let mut connection = self.connection.clone();
        let raw: Vec<String> = connection
            .lrange(self.events_key(run_id), 0, -1)
            .await
            .map_err(|err| redis_error("read events", err))?;
        raw.iter().map(|entry| decode(entry)).collect()
    }

    async fn publish(&self, run_id: RunId, notification: Notification) -> StoreResult<()> {
        let mut connection = self.connection.clone();
        // No subscribers is not an error: the event log is the durable record.
        let _: () = connection
            .publish(self.channel(run_id), encode(&notification)?)
            .await
            .map_err(|err| redis_error("publish notification", err))?;
        Ok(())
    }

    async fn subscribe(&self, run_id: RunId) -> StoreResult<NotificationStream> {
        let mut pubsub = self
            .client
            .get_async_pubsub()
            .await
            .map_err(|err| redis_error("open Redis pub/sub connection", err))?;
        // Returns only once Redis has acknowledged the subscription, so a
        // caller can subscribe and then act without missing its own effects.
        pubsub
            .subscribe(self.channel(run_id))
            .await
            .map_err(|err| redis_error("subscribe to run channel", err))?;

        let stream = pubsub.into_on_message().filter_map(|message| async move {
            let raw = message.get_payload::<String>().ok()?;
            match decode::<Notification>(&raw) {
                Ok(notification) => Some(notification),
                Err(error) => {
                    tracing::warn!(%error, "dropping undecodable notification");
                    None
                }
            }
        });
        Ok(Box::pin(stream))
    }

    async fn get_session(&self, session_id: SessionId) -> StoreResult<Option<SessionRecord>> {
        let mut connection = self.connection.clone();
        let raw: Option<String> = connection
            .get(self.session_meta_key(session_id))
            .await
            .map_err(|err| redis_error("read session", err))?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        let meta: SessionMeta = decode(&raw)?;

        let encoded: Vec<String> = connection
            .lrange(self.session_messages_key(session_id), 0, -1)
            .await
            .map_err(|err| redis_error("read session messages", err))?;
        let messages: Vec<Message> =
            encoded.iter().map(|entry| decode(entry)).collect::<StoreResult<_>>()?;

        // Rebuild history: entries hosted elsewhere first, then one URL per
        // message this deployment stores.
        let base_url = meta.base_url.as_deref().unwrap_or_default();
        let mut history = meta.prefix_history.clone();
        history.extend((0..messages.len()).map(|index| message_url(base_url, session_id, index)));

        Ok(Some(SessionRecord {
            session: Session { id: meta.id, history, state: meta.state },
            messages,
        }))
    }

    async fn ensure_session(&self, session: Session) -> StoreResult<SessionRecord> {
        let key = self.session_meta_key(session.id);
        let meta = SessionMeta {
            id: session.id,
            state: session.state.clone(),
            prefix_history: session.history.clone(),
            base_url: None,
        };

        let mut connection = self.connection.clone();
        // SET NX: whichever replica gets there first seeds the session; the
        // rest read what is already stored.
        let created: bool = connection
            .set_nx(&key, encode(&meta)?)
            .await
            .map_err(|err| redis_error("create session", err))?;
        if created {
            self.touch(&key).await?;
        }

        self.get_session(session.id).await?.ok_or_else(|| {
            Error::server_error(format!(
                "session {} vanished immediately after creation",
                session.id
            ))
        })
    }

    async fn append_session_messages(
        &self,
        session_id: SessionId,
        base_url: &str,
        messages: Vec<Message>,
    ) -> StoreResult<()> {
        if messages.is_empty() {
            return Ok(());
        }
        let meta_key = self.session_meta_key(session_id);
        let messages_key = self.session_messages_key(session_id);
        let mut connection = self.connection.clone();

        // Ensure the session exists and knows the base URL its history links
        // resolve against.
        let raw: Option<String> =
            connection.get(&meta_key).await.map_err(|err| redis_error("read session", err))?;
        let mut meta: SessionMeta = match raw.as_deref() {
            Some(raw) => decode(raw)?,
            None => SessionMeta {
                id: session_id,
                state: None,
                prefix_history: Vec::new(),
                base_url: None,
            },
        };
        if meta.base_url.as_deref() != Some(base_url) {
            meta.base_url = Some(base_url.to_string());
            let _: () = connection
                .set(&meta_key, encode(&meta)?)
                .await
                .map_err(|err| redis_error("write session", err))?;
        }

        // A single RPUSH keeps concurrent appends from interleaving, and the
        // index each message lands at is exactly what its history URL uses.
        let encoded: Vec<String> =
            messages.iter().map(encode).collect::<StoreResult<Vec<String>>>()?;
        let _: () = connection
            .rpush(&messages_key, encoded)
            .await
            .map_err(|err| redis_error("append session messages", err))?;

        self.touch(&meta_key).await?;
        self.touch(&messages_key).await
    }

    async fn get_session_state(
        &self,
        session_id: SessionId,
    ) -> StoreResult<Option<serde_json::Value>> {
        let mut connection = self.connection.clone();
        let raw: Option<String> = connection
            .get(self.session_state_key(session_id))
            .await
            .map_err(|err| redis_error("read session state", err))?;
        raw.as_deref().map(decode).transpose()
    }

    async fn put_session_state(
        &self,
        session_id: SessionId,
        base_url: &str,
        state: serde_json::Value,
    ) -> StoreResult<()> {
        let state_key = self.session_state_key(session_id);
        let meta_key = self.session_meta_key(session_id);
        let mut connection = self.connection.clone();

        let _: () = connection
            .set(&state_key, encode(&state)?)
            .await
            .map_err(|err| redis_error("write session state", err))?;

        // Record the link on the session, creating it if this is the first
        // thing written for it.
        let raw: Option<String> =
            connection.get(&meta_key).await.map_err(|err| redis_error("read session", err))?;
        let mut meta: SessionMeta = match raw.as_deref() {
            Some(raw) => decode(raw)?,
            None => SessionMeta {
                id: session_id,
                state: None,
                prefix_history: Vec::new(),
                base_url: None,
            },
        };
        meta.state = Some(state_url(base_url, session_id));
        let _: () = connection
            .set(&meta_key, encode(&meta)?)
            .await
            .map_err(|err| redis_error("write session", err))?;

        self.touch(&state_key).await?;
        self.touch(&meta_key).await
    }
}

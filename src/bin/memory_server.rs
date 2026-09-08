//! A minimal, real server binary for the `Memory` domain — wraps a
//! `GenericProductionStore<MemoryProductionStack>` seeded from a small,
//! hand-written sample dataset in a `MemoryConnectionStore` and serves
//! it over TCP. Mirrors `dog_server.rs`'s own shape and env-var reading
//! exactly (`MEM-FR-007`, ADR-0048) — every operational knob `dog_server`
//! exposes is exposed here too, unchanged. See
//! `rusty_multimodal_db::server`'s own module docs for the protocol and
//! what this deliberately does not provide.
//!
//! # This is a local development tool, not a deployable service
//!
//! Authentication/authorization (`ServeOptions::from_env`, ADR-0012) and
//! native transport encryption (`TlsConfig::from_env`, ADR-0014) are both
//! real and opt-in via the process environment
//! (`SERVER_AUTH_READ_ONLY_TOKEN`/`SERVER_AUTH_READ_WRITE_TOKEN`,
//! `SERVER_TLS_CERT_CHAIN_PATH`/`SERVER_TLS_PRIVATE_KEY_PATH`, and —
//! for mutual TLS, ADR-0023 — an optional `SERVER_TLS_CLIENT_CA_PATH`
//! naming the CA roots every client certificate must chain to; and, since
//! ADR-0028, `SERVER_AUTH_READ_ONLY_CLIENT_CERTS`/
//! `SERVER_AUTH_READ_WRITE_CLIENT_CERTS` (`:`-separated PEM files)
//! classing a presented certificate by exact match — refused at startup
//! without `SERVER_TLS_CLIENT_CA_PATH` (`CLS-FR-005`); and, since
//! ADR-0030, an opt-in per-peer failed-`Authenticate` budget,
//! `SERVER_AUTH_RATE_LIMIT="<failures>/<seconds>"` (a per-connection
//! lockout after five failures is on by default, not configurable); and
//! `SERVER_TXN_JOURNAL_PATH`, ADR-0025, making every transaction batch
//! crash-atomic at one `fsync` per batch, shared across concurrent batches
//! by ADR-0026's group commit; and, since ADR-0031, an opt-in per-request
//! access log independent of the audit log, `SERVER_ACCESS_LOG` (`stderr`
//! or a file path)) — with none of them set, this behaves exactly as it
//! did before any of these features existed: no auth, no encryption, no
//! journal, no audit log (`SERVER_AUDIT_LOG`, ADR-0029: `stderr` or a file
//! path), no access log. Do not expose this beyond a trusted,
//! localhost/development network unless both auth and TLS are configured
//! — see ADR-0010's Consequences. Usage: `memory_server [host:port]`
//! (defaults to `127.0.0.1:7881`).
//!
//! # Where the data lives — `SERVER_DATA_DIR` (ADR-0053)
//!
//! Unset, this binary seeds the sample dataset into a fresh per-process
//! scratch directory on every start — a demonstration, nothing survives
//! a restart. Set `SERVER_DATA_DIR=<dir>` and it becomes a durable
//! backend (`DDR-FR-002`): the directory is created if missing;
//! `<dir>/memories.mmap` and `<dir>/entities.mmap` are each **opened
//! from their files alone** when present and created **empty** when
//! not (`open_or_create_*_production_stack`) — no sample data, and no
//! caller-supplied record list that could overrule what earlier runs
//! inserted, replaced, linked, deleted, or compacted. A directory that
//! holds a slot file but lacks a companion is a startup error, never a
//! silent recreation. Combine with `SERVER_TXN_JOURNAL_PATH` for
//! crash-atomic batches; the two are independent settings.

use rusty_multimodal_db::generic::entity::{
    create_entity_production_stack, open_or_create_entity_production_stack, Entity,
    EntityProductionStack,
};
use rusty_multimodal_db::generic::memory::{
    create_memory_production_stack, open_or_create_memory_production_stack, Memory,
    MemoryProductionStack,
};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::access::{AccessSink, FileAccessLog, StderrAccessLog};
use rusty_multimodal_db::server::audit::{AuditSink, FileAudit, StderrAudit};
use rusty_multimodal_db::server::entity::EntityConnectionStore;
use rusty_multimodal_db::server::memory::MemoryConnectionStore;
use rusty_multimodal_db::server::{
    serve_tables, ConnectionStore, RateLimit, ServeOptions, TlsConfig, TokenClass,
};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

fn sample_memories() -> Vec<Memory> {
    let memory = |n: u128, content: &str, category: &str| Memory {
        id: Uuid::from_u128(n),
        content: content.into(),
        category: category.into(),
        tags: vec!["sample".into()],
        source: "manual".into(),
        metadata_json: "{}".into(),
        created_at_unix_ms: 1_000 * n as i64,
        updated_at_unix_ms: 1_000 * n as i64,
        memory_type: "unclassified".into(),
        status: "active".into(),
        sensitive: false,
        access_count: 0,
        deleted_at_unix_ms: 0,
        node_id: String::new(),
    };
    vec![
        memory(1, "Prefers merge commits over squash", "preference"),
        memory(
            2,
            "The store's record set is no longer fixed at open",
            "fact",
        ),
        memory(
            3,
            "Decided: open relation labels, with a manifest",
            "decision",
        ),
    ]
}

/// `TBL-FR-001` (ADR-0050): the second table — three entities the sample
/// memories mention, served beside `memory` on the same listener.
fn sample_entities() -> Vec<Entity> {
    let entity = |n: u128, label: &str, kind: &str| Entity {
        id: Uuid::from_u128(n),
        label: label.into(),
        kind: kind.into(),
        mention_count: 0,
        aliases: vec![],
    };
    vec![
        entity(0xada, "Ada Lovelace", "person"),
        entity(0x46, "ADR-0046", "decision"),
        entity(0x47, "ADR-0047", "decision"),
    ]
}

/// Which sample memory mentions which sample entity — `(memory, entity)`.
fn sample_mentions() -> Vec<(Uuid, Uuid)> {
    vec![
        (Uuid::from_u128(2), Uuid::from_u128(0x46)),
        (Uuid::from_u128(3), Uuid::from_u128(0x47)),
    ]
}

fn main() {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:7881".to_string());

    // `SERVER_DATA_DIR` (ADR-0053, `DDR-FR-002`): set, a durable
    // directory opened from its files or created empty; unset, the
    // sample dataset in a per-process scratch directory, as before.
    let data = match std::env::var_os("SERVER_DATA_DIR") {
        Some(dir) => DataLocation::Durable(PathBuf::from(dir)),
        None => DataLocation::Scratch(
            std::env::temp_dir().join(format!("memory_server_{}", std::process::id())),
        ),
    };
    let (store, entity_store) = open_stores(&data).unwrap_or_else(|e| panic!("{e}"));
    let entity_connection_store: Arc<dyn ConnectionStore> = Arc::new(EntityConnectionStore::new(
        GenericProductionStore::new(entity_store),
    ));
    // `SERVER_TXN_JOURNAL_PATH` (ADR-0025): with it, every transaction
    // batch is crash-atomic — journaled and fsync'd before its first
    // write, replayed on the next start. Set it the same way every start:
    // opening without it after a crash forgoes the replay.
    let connection_store = Arc::new(match std::env::var("SERVER_TXN_JOURNAL_PATH") {
        Ok(journal_path) => MemoryConnectionStore::with_journal(
            GenericProductionStore::new(store),
            Path::new(&journal_path),
        )
        .unwrap_or_else(|e| panic!("SERVER_TXN_JOURNAL_PATH configured but invalid: {e}")),
        Err(_) => MemoryConnectionStore::new(GenericProductionStore::new(store)),
    });
    let journaled = std::env::var_os("SERVER_TXN_JOURNAL_PATH").is_some();

    let listener = TcpListener::bind(&addr).unwrap_or_else(|e| panic!("binding {addr}: {e}"));

    // `SERVER_AUDIT_LOG` (ADR-0029): `stderr`, or a file path appended to;
    // unset → no audit. An unopenable path is a startup error.
    let auth = match audit_sink_from(std::env::var("SERVER_AUDIT_LOG").ok().as_deref()) {
        Ok(Some(sink)) => ServeOptions::from_env().with_audit(sink),
        Ok(None) => ServeOptions::from_env(),
        Err(e) => panic!("SERVER_AUDIT_LOG configured but invalid: {e}"),
    };
    // `SERVER_AUTH_READ_ONLY_CLIENT_CERTS`/`SERVER_AUTH_READ_WRITE_CLIENT_CERTS`
    // (ADR-0028, `CLS-FR-005`): certificate-classed connections.
    let auth = certificate_classes_from_env_values(
        auth,
        std::env::var("SERVER_AUTH_READ_ONLY_CLIENT_CERTS")
            .ok()
            .as_deref(),
        std::env::var("SERVER_AUTH_READ_WRITE_CLIENT_CERTS")
            .ok()
            .as_deref(),
        std::env::var("SERVER_TLS_CLIENT_CA_PATH").ok().as_deref(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    // `SERVER_AUTH_RATE_LIMIT` (ADR-0030, `RL-FR-006`): an opt-in
    // per-peer failed-`Authenticate` budget; the per-connection lockout
    // at `MAX_AUTH_FAILURES` is always on and has no variable.
    let auth = rate_limit_from_env_value(
        auth,
        std::env::var("SERVER_AUTH_RATE_LIMIT").ok().as_deref(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let rate_limited = auth.rate_limit().is_some();
    // `SERVER_ACCESS_LOG` (ADR-0031, `ACC-FR-007`): `stderr`, or a file
    // path appended to; unset → no access log. Independent of
    // `SERVER_AUDIT_LOG` — an unopenable path is a startup error.
    let auth = match access_sink_from(std::env::var("SERVER_ACCESS_LOG").ok().as_deref()) {
        Ok(Some(sink)) => auth.with_access_log(sink),
        Ok(None) => auth,
        Err(e) => panic!("SERVER_ACCESS_LOG configured but invalid: {e}"),
    };
    let access_logged = std::env::var_os("SERVER_ACCESS_LOG").is_some();
    let audited = std::env::var_os("SERVER_AUDIT_LOG").is_some();
    let tls = match TlsConfig::from_env() {
        None => None,
        Some(Ok(tls)) => Some(tls),
        Some(Err(e)) => panic!(
            "SERVER_TLS_CERT_CHAIN_PATH/SERVER_TLS_PRIVATE_KEY_PATH/SERVER_TLS_CLIENT_CA_PATH configured but invalid: {e}"
        ),
    };
    // `SRV-FR-003` (ADR-0032): `TlsConfig` stays its own separately-fallible
    // construction step, folded in only after its `Result` is handled above.
    let options = match tls {
        Some(tls) => auth.with_tls(tls),
        None => auth,
    };
    eprintln!(
        "memory_server listening on {addr} (data: {}, auth: {}, TLS: {}, transaction journal: {}, audit log: {}, auth rate limit: {}, access log: {} — see ADR-0012/ADR-0014/ADR-0023/ADR-0025/ADR-0029/ADR-0030/ADR-0031/ADR-0048/ADR-0053; do not expose beyond a trusted network unless auth and TLS are both configured)",
        data.describe(),
        if options.is_configured() { "configured" } else { "NOT configured" },
        match options.tls() {
            None => "NOT configured",
            Some(tls) if tls.requires_client_certificate() => "configured, client certificate required",
            Some(_) => "configured",
        },
        if journaled { "configured" } else { "NOT configured" },
        if audited { "configured" } else { "NOT configured" },
        if rate_limited { "configured" } else { "lockout only (default)" },
        if access_logged { "configured" } else { "NOT configured" },
    );

    // `TBL-FR-001` (ADR-0050): two tables on one listener, `memory`
    // primary — `Use entity` reaches the other; `JOIN entity e ON
    // mentions` crosses between them.
    let memory_connection_store: Arc<dyn ConnectionStore> = connection_store;
    serve_tables(
        listener,
        vec![
            ("memory".to_string(), memory_connection_store),
            ("entity".to_string(), entity_connection_store),
        ],
        0,
        options,
    );
}

/// Where this process keeps its two tables (`DDR-FR-002`).
enum DataLocation {
    /// `SERVER_DATA_DIR`: opened from the files when present, created
    /// empty when not; survives restarts.
    Durable(PathBuf),
    /// No `SERVER_DATA_DIR`: the sample dataset, recreated every start.
    Scratch(PathBuf),
}

impl DataLocation {
    fn describe(&self) -> String {
        match self {
            Self::Durable(dir) => format!("durable at {}", dir.display()),
            Self::Scratch(dir) => {
                format!("sample data, scratch at {} (NOT durable)", dir.display())
            }
        }
    }
}

/// `SERVER_DATA_DIR`'s decision table (`DDR-FR-002`): the directory is
/// created if missing; durable tables open-or-create, scratch tables
/// are seeded. Every failure names the directory.
fn open_stores(
    data: &DataLocation,
) -> Result<(MemoryProductionStack, EntityProductionStack), String> {
    let (dir, durable) = match data {
        DataLocation::Durable(dir) => (dir, true),
        DataLocation::Scratch(dir) => (dir, false),
    };
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("creating the data directory {}: {e}", dir.display()))?;
    let memories = dir.join("memories.mmap");
    let entities = dir.join("entities.mmap");
    if durable {
        let store = open_or_create_memory_production_stack(&memories)
            .map_err(|e| format!("SERVER_DATA_DIR: opening {}: {e}", memories.display()))?;
        let entity_store = open_or_create_entity_production_stack(&entities)
            .map_err(|e| format!("SERVER_DATA_DIR: opening {}: {e}", entities.display()))?;
        return Ok((store, entity_store));
    }
    let store = create_memory_production_stack(sample_memories(), &sample_mentions(), &memories)
        .map_err(|e| format!("creating the sample MemoryProductionStack: {e}"))?;
    let entity_store = create_entity_production_stack(sample_entities(), &[], &[], &entities)
        .map_err(|e| format!("creating the sample EntityProductionStack: {e}"))?;
    Ok((store, entity_store))
}

/// `SERVER_AUDIT_LOG`'s decision table (`AUD-FR-008`) — see
/// `dog_server`'s identical function for the full contract.
fn audit_sink_from(value: Option<&str>) -> std::io::Result<Option<Arc<dyn AuditSink>>> {
    match value {
        None => Ok(None),
        Some("stderr") => Ok(Some(Arc::new(StderrAudit::new()))),
        Some(path) => Ok(Some(Arc::new(FileAudit::open(Path::new(path))?))),
    }
}

/// `SERVER_ACCESS_LOG`'s decision table (`ACC-FR-007`) — see
/// `dog_server`'s identical function for the full contract.
fn access_sink_from(value: Option<&str>) -> std::io::Result<Option<Arc<dyn AccessSink>>> {
    match value {
        None => Ok(None),
        Some("stderr") => Ok(Some(Arc::new(StderrAccessLog::new()))),
        Some(path) => Ok(Some(Arc::new(FileAccessLog::open(Path::new(path))?))),
    }
}

/// `SERVER_AUTH_READ_ONLY_CLIENT_CERTS`/`SERVER_AUTH_READ_WRITE_CLIENT_CERTS`'s
/// decision table (`CLS-FR-005`, ADR-0028) — see `dog_server`'s identical
/// function for the full contract.
fn certificate_classes_from_env_values(
    mut auth: ServeOptions,
    read_only_certs: Option<&str>,
    read_write_certs: Option<&str>,
    client_ca_path: Option<&str>,
) -> Result<ServeOptions, String> {
    if (read_only_certs.is_some() || read_write_certs.is_some()) && client_ca_path.is_none() {
        return Err(
            "SERVER_AUTH_READ_ONLY_CLIENT_CERTS/SERVER_AUTH_READ_WRITE_CLIENT_CERTS is set but \
             SERVER_TLS_CLIENT_CA_PATH is not — a certificate class map is inert without a \
             client-certificate-requiring TLS config"
                .to_string(),
        );
    }
    for (variable, paths, class) in [
        (
            "SERVER_AUTH_READ_ONLY_CLIENT_CERTS",
            read_only_certs,
            TokenClass::ReadOnly,
        ),
        (
            "SERVER_AUTH_READ_WRITE_CLIENT_CERTS",
            read_write_certs,
            TokenClass::ReadWrite,
        ),
    ] {
        if let Some(paths) = paths {
            for path in paths.split(':') {
                auth = auth
                    .with_certificate_class_pem_file(path, class)
                    .map_err(|e| format!("{variable} configured but invalid: {e}"))?;
            }
        }
    }
    Ok(auth)
}

/// `SERVER_AUTH_RATE_LIMIT`'s decision table (`RL-FR-006`) — see
/// `dog_server`'s identical function for the full contract.
fn rate_limit_from_env_value(
    auth: ServeOptions,
    value: Option<&str>,
) -> Result<ServeOptions, String> {
    match value {
        None => Ok(auth),
        Some(value) => RateLimit::parse(value)
            .map(|limit| auth.with_rate_limit(limit))
            .map_err(|e| format!("SERVER_AUTH_RATE_LIMIT configured but invalid: {e}")),
    }
}

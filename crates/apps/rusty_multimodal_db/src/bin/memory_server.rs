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
//! `<dir>/memories.mmap`, `<dir>/entities.mmap`, and `<dir>/relations.mmap`
//! (`ADR-0058`, the third table) are each **opened
//! from their files alone** when present and created **empty** when
//! not (`open_or_create_*_production_stack`) — no sample data, and no
//! caller-supplied record list that could overrule what earlier runs
//! inserted, replaced, linked, deleted, or compacted. A directory that
//! holds a slot file but lacks a companion is a startup error, never a
//! silent recreation. Combine with `SERVER_TXN_JOURNAL_PATH` for
//! crash-atomic batches; the two are independent settings.
//!
//! **One process per directory (`ADR-0092`).** Before any of the three
//! files is opened, this binary claims `<dir>/.rusty_multimodal_db.lock`
//! with an exclusive advisory lock (`server::data_lock::DataDirLock`)
//! and holds it until it exits; a second `memory_server` pointed at
//! the same directory refuses to start, naming that file. The kernel
//! releases the lock however the process ends, so a killed server
//! leaves nothing to clean up.
//!
//! # Connection limits — `SERVER_IDLE_TIMEOUT_SECS`, `SERVER_MAX_CONNECTIONS`, `SERVER_MAX_QUERY_ROWS` (ADR-0093)
//!
//! Since `ADR-0099`/`ADR-0102` all three default on: unset,
//! `SERVER_IDLE_TIMEOUT_SECS` is 300, `SERVER_MAX_CONNECTIONS` is 1024,
//! and `SERVER_MAX_QUERY_ROWS` is 10000; `0` turns any of them off.
//! `SERVER_IDLE_TIMEOUT_SECS=<n>` closes a connection that
//! neither sends nor accepts a byte for `n` seconds (its session, if
//! any, rolls back and its MVCC snapshot is released, exactly as on a
//! disconnect). `SERVER_MAX_CONNECTIONS=<n>` closes the `n+1`th
//! concurrent connection at accept, before any byte is read, and
//! counts it in `dogserver_connections_refused_total`.
//! `SERVER_MAX_QUERY_ROWS=<n>` answers a `Query` with no `limit` as if it
//! had asked for `n` rows (counted in `dogserver_query_rows_clamped_total`,
//! ADR-0102), and refuses a `Query` or page whose `limit` is above `n`
//! with `TooLarge` before any read. A value that is not a non-negative
//! integer is a startup error.
//!
//! # Durable acknowledgements — `SERVER_SYNC_UPDATES` (ADR-0097)
//!
//! An `Insert`/`Replace`/`Delete`/`Link` is `fsync`ed to the insert log
//! before it is acknowledged, and a journaled `Transaction` batch
//! (`SERVER_TXN_JOURNAL_PATH`) is `fsync`ed to the journal first. An
//! `UpdateField`, and a `Transaction` batch on a table with no
//! journal, is an in-place write to a mapped page: acknowledged at
//! once, on disk at the next `Flush`, checkpoint, or OS write-back —
//! it survives a process crash, not a power loss. Since `ADR-0099`
//! those two paths `msync` the table's slot files before acknowledging
//! by default, at roughly the cost of one `fsync` per update
//! (`RESULTS.md`: ~100 µs); `SERVER_SYNC_UPDATES=0` turns that off and
//! takes the write-back window back.
//!
//! # Updates through the journal — `SERVER_JOURNAL_UPDATES` (ADR-0107)
//!
//! With `SERVER_TXN_JOURNAL_PATH` set, `SERVER_JOURNAL_UPDATES=1` commits
//! every `UpdateField` through the journal as a one-operation batch —
//! the redo entry `fsync`ed before the slot is written, replayed on the
//! next start — instead of `msync`ing the table's mapping. Durable past
//! a power loss either way; the cost differs: for one writer the journal
//! path is slower on a small table (~220 vs ~145 µs) and faster on a
//! large one (~240 vs ~310 µs at 1M rows); under eight concurrent
//! writers one `fsync` covers a group and the per-update cost halves
//! (~90 vs ~180 µs) — `RESULTS.md`. Off by default; an error without a
//! journal.
//!
//! # Journal and MVCC together (ADR-0114)
//!
//! `SERVER_TXN_JOURNAL_PATH` and `SERVER_MVCC_ISOLATION` may both be
//! set (until ADR-0114 the pair was refused at startup). A reopened
//! `memory` table then reads its pending insert log before the reopen
//! clears it, replays the journal, and folds both into the reconstructed
//! version index before serving, so a fresh snapshot after a restart
//! sees every batch the journal acknowledged.
//!
//! # MVCC history bounded — `SERVER_MVCC_RECLAIM_EVERY` (ADR-0105)
//!
//! With `SERVER_MVCC_ISOLATION` set, every committed write appends a
//! version entry that some open snapshot might still need. `ADR-0096`
//! reclaimed the entries no open snapshot needs at `Compact` only; by
//! default the server now also reclaims them on its own every 10,000
//! appended entries (`SERVER_MVCC_RECLAIM_EVERY`; `0` leaves it to
//! `Compact`), inside the same lock the write took, at the oldest
//! open snapshot — so a deployment that never compacts no longer
//! holds every version ever written.
//!
//! # Live backups — `SERVER_BACKUP_ROOT` (ADR-0065)
//!
//! Opt-in, and only meaningful with `SERVER_DATA_DIR` also set (a
//! scratch-mode table has nothing worth backing up — it is gone on
//! restart regardless). Set `SERVER_BACKUP_ROOT=<dir>` and an
//! authenticated write-class client's `Request::Backup { name }`
//! copies one table's files, under its write lock, into
//! `<dir>/<name>` — `name` a single path component, never a caller
//! path. Unset, every `Backup` request answers `Unsupported`.
//!
//! # Process metrics — `Request::Metrics` (ADR-0064)
//!
//! Always on, no configuration: any authenticated connection may call
//! `Request::Metrics` for a bounded set of process-wide counters as
//! Prometheus text.
//!
//! # Replication snapshots — `SERVER_AUTH_REPLICATION_TOKEN` (ADR-0067)
//!
//! Opt-in, and only meaningful with `SERVER_BACKUP_ROOT`'s own
//! prerequisite — a durable table (`SERVER_DATA_DIR`) — also true (the
//! same file set `Request::Backup` would copy). Set
//! `SERVER_AUTH_REPLICATION_TOKEN=<token>` and a connection presenting
//! it authenticates at `TokenClass::Replication`, the *only* class
//! `Request::FetchSnapshot` accepts — never granted by
//! `SERVER_AUTH_READ_WRITE_TOKEN`/`SERVER_AUTH_READ_ONLY_TOKEN`, even
//! together. `FetchSnapshot` streams every file this table owns back
//! over the connection, up to `MAX_SNAPSHOT_BYTES`. Unset, every
//! `FetchSnapshot` request answers `Unauthorized`.
//!
//! # Exposure — a non-loopback bind needs auth and TLS (ADR-0094)
//!
//! Binding anything but a loopback address (`127.0.0.1`, `[::1]`) with
//! no authentication configured, or with authentication but no TLS,
//! refuses to start and says which is missing. `SERVER_ALLOW_INSECURE=1`
//! turns that refusal into a warning for an operator who means it. A
//! loopback bind needs nothing, as every version before.
//!
//! # Metrics scrape — `SERVER_METRICS_HTTP_ADDR` (ADR-0069, ADR-0111)
//!
//! Set to `host:port` to bind a second listener answering a plain
//! `GET /metrics` with the same Prometheus text `Request::Metrics`
//! carries; unset, no listener. It has no authentication of its own, so
//! a non-loopback address is refused at startup (`RVM-FR-004`) unless
//! `SERVER_ALLOW_INSECURE=1`, which turns the refusal into a warning.

use rusty_multimodal_db::generic::entity::{
    create_entity_production_stack, open_or_create_entity_production_stack, Entity,
    EntityProductionStack,
};
use rusty_multimodal_db::generic::memory::{
    create_memory_production_stack, open_or_create_memory_production_stack, Memory,
    MemoryProductionStack,
};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::generic::relation::{
    create_relation_production_stack, open_or_create_relation_production_stack, Relation,
    RelationProductionStack,
};
use rusty_multimodal_db::server::access::{AccessSink, FileAccessLog, StderrAccessLog};
use rusty_multimodal_db::server::audit::{AuditSink, FileAudit, StderrAudit};
use rusty_multimodal_db::server::data_lock::DataDirLock;
use rusty_multimodal_db::server::entity::EntityConnectionStore;
use rusty_multimodal_db::server::exposure::{
    allow_insecure_from_env, check_exposure, check_metrics_exposure,
};
use rusty_multimodal_db::server::memory::MemoryConnectionStore;
use rusty_multimodal_db::server::relation::RelationConnectionStore;
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
/// `REL-FR-005` (ADR-0058): the third table — two directed edges between
/// the sample entities, by the consumer's own id strings.
fn sample_relations() -> Vec<Relation> {
    let relation = |n: u128, subject: &str, label: &str, object: &str| Relation {
        id: Uuid::from_u128(n),
        subject: subject.into(),
        relation: label.into(),
        object: object.into(),
        created_at_unix_ms: 1_000 * n as i64,
        updated_at_unix_ms: 1_000 * n as i64,
        node_id: String::new(),
        deleted_at_unix_ms: 0,
    };
    vec![
        relation(1, "ada", "authored", "adr-0046"),
        relation(2, "adr-0047", "follows", "adr-0046"),
    ]
}

fn sample_mentions() -> Vec<(Uuid, Uuid)> {
    vec![
        (Uuid::from_u128(2), Uuid::from_u128(0x46)),
        (Uuid::from_u128(3), Uuid::from_u128(0x47)),
    ]
}

/// `LIM-FR-005` (ADR-0093): an opt-in positive-integer setting — `None`
/// when unset; a startup error when set to anything else.
/// `LIM-FR-005` (ADR-0093) as amended by `DEF-FR-001` (ADR-0099): a
/// bounded setting with a default — unset is `default`; `0` is "off"
/// (`None`); a positive integer is itself; anything else is a startup
/// error.
fn bounded_env(name: &str, default: Option<u64>) -> Option<u64> {
    let Ok(raw) = std::env::var(name) else {
        return default;
    };
    // `RVL-FR-005` (ADR-0112): an exported-but-empty variable is unset.
    if raw.trim().is_empty() {
        return default;
    }
    match raw.trim().parse::<u64>() {
        Ok(0) => None,
        Ok(n) => Some(n),
        Err(e) if *e.kind() == std::num::IntErrorKind::PosOverflow => {
            panic!("{name}={raw:?} is larger than a 64-bit integer allows")
        }
        Err(_) => panic!("{name}={raw:?} is not a non-negative integer (0 turns the limit off)"),
    }
}

/// `DEF-FR-002` (ADR-0099): an on/off setting that defaults to on —
/// unset or `1` is on, `0` is off, anything else a startup error.
fn switch_env(name: &str) -> bool {
    switch_env_with_default(name, true)
}

/// `RVL-FR-005` (ADR-0112): an on/off setting with a stated default —
/// unset or empty is the default, `1` on, `0` off, anything else a
/// startup error.
fn switch_env_with_default(name: &str, default: bool) -> bool {
    let Ok(raw) = std::env::var(name) else {
        return default;
    };
    match raw.trim() {
        "" => default,
        "1" => true,
        "0" => false,
        _ => panic!("{name}={raw:?} is neither 0 nor 1"),
    }
}

/// `DEF-FR-001` (ADR-0099): the idle timeout a deployment gets without
/// asking — five minutes, long past any request's round trip and short
/// enough that an abandoned session's MVCC snapshot is released the
/// same hour.
const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 300;
/// `DEF-FR-001` (ADR-0099): the connection cap a deployment gets without
/// asking — a thread each, well under a Linux default thread limit.
const DEFAULT_MAX_CONNECTIONS: u64 = 1024;
/// `CLP-FR-003` (ADR-0102): the row cap a deployment gets without asking
/// — a `Query` with no `limit` is answered as if it had asked for this
/// many rows (and counted); an explicit `limit` above it is refused.
const DEFAULT_MAX_QUERY_ROWS: u64 = 10_000;
/// `ART-FR-004` (ADR-0105): with MVCC on, reclaim history every this
/// many appended entries without waiting for a `Compact` — a bound on
/// the version index a deployment gets without asking.
const DEFAULT_MVCC_RECLAIM_EVERY: u64 = 10_000;

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
    let durable = matches!(data, DataLocation::Durable(_));
    // `ADR-0092` (`DDL-FR-002`): one process per data directory. Taken
    // before any store is opened, held until `main` returns; a second
    // server on the same directory refuses to start, naming the holder's
    // lock file. Scratch mode is a per-process path nobody else can
    // share, so it takes no lock.
    let _data_dir_lock = match &data {
        DataLocation::Durable(dir) => Some(DataDirLock::acquire(dir).unwrap_or_else(|e| {
            panic!("SERVER_DATA_DIR {dir:?}: {e} — one memory_server per data directory")
        })),
        DataLocation::Scratch(_) => None,
    };
    let journaled = std::env::var_os("SERVER_TXN_JOURNAL_PATH").is_some();
    // `SERVER_JOURNAL_UPDATES` (`ADR-0107`, `JUF-FR-003`): with a journal,
    // every `UpdateField` is committed through it (group commit) instead
    // of `msync`ing the mapping; a `0`/`1` switch, off by default
    // (`RVL-FR-005`, ADR-0112), an error without `SERVER_TXN_JOURNAL_PATH`.
    let journal_updates = switch_env_with_default("SERVER_JOURNAL_UPDATES", false);
    if journal_updates && !journaled {
        panic!("SERVER_JOURNAL_UPDATES needs SERVER_TXN_JOURNAL_PATH: there is no journal to commit through");
    }
    // `SERVER_MVCC_ISOLATION` (`ADR-0072`, `MVCC2-FR-004`/`011`): opt-in,
    // unset by default — matching `with_mvcc`/`open_with_mvcc`'s own
    // "unset by default" framing and `SERVER_BACKUP_ROOT`/
    // `SERVER_AUTH_REPLICATION_TOKEN`'s own presence-gated precedent.
    // With it set, a session may open with `SESSION_MVCC_ISOLATION` on
    // any of the three tables; unset, that bit is refused `Unsupported`
    // everywhere, exactly as it always was before this variable existed.
    let mvcc_enabled = std::env::var_os("SERVER_MVCC_ISOLATION").is_some();
    // `SERVER_SYNC_UPDATES` (`ADR-0097`, `SYU-FR-004`; on by default since
    // `ADR-0099`, `DEF-FR-002`): every `UpdateField` and every
    // non-journaled `Transaction` batch is `msync`ed before it is
    // acknowledged; `0` turns it off and the acknowledgement precedes
    // durability by the OS's write-back.
    let sync_updates = switch_env("SERVER_SYNC_UPDATES");
    // `SERVER_MVCC_RECLAIM_EVERY` (`ADR-0105`, `ART-FR-004`): with MVCC
    // on, history no open snapshot needs is reclaimed automatically
    // every this many appended entries; `0` leaves it to `Compact`.
    let mvcc_reclaim_every = bounded_env(
        "SERVER_MVCC_RECLAIM_EVERY",
        Some(DEFAULT_MVCC_RECLAIM_EVERY),
    )
    .map(|n| usize::try_from(n).unwrap_or(usize::MAX));
    // `JMC-FR-003` (ADR-0114): `SERVER_TXN_JOURNAL_PATH` and
    // `SERVER_MVCC_ISOLATION` compose — a reopened `memory` table takes
    // `open_with_mvcc_journaled` below, which reads the pending insert
    // log before the reopen clears it, replays the journal, and folds
    // both into the reconstructed index. Until ADR-0114 the pair was
    // refused at startup.
    // `JMC-FR-003` (ADR-0114): a table an MVCC constructor will reopen
    // is not opened here first — `open_with_mvcc`/`open_with_mvcc_journaled`
    // must read the insert log before any open of the file folds and
    // clears it. Through ADR-0113 this binary opened every table once
    // through `open_stores` and then a second time in the MVCC reopen;
    // the first, unused mapping had already cleared the log, so a
    // journaled batch pending in it never reached the index (an
    // ordinary write survived only because it flushes the history
    // itself). `open_stores` now leaves such a table to its adapter.
    let OpenedStores {
        store,
        entity_store,
        relation_store,
        memories_path,
        entities_path,
        relations_path,
    } = open_stores(&data, mvcc_enabled).unwrap_or_else(|e| panic!("{e}"));
    let relation_connection_store: Arc<dyn ConnectionStore> = Arc::new({
        let adapter = match relation_store {
            TableOpen::ReopenWithMvcc => RelationConnectionStore::open_with_mvcc(&relations_path)
                .unwrap_or_else(|e| {
                    panic!(
                        "SERVER_MVCC_ISOLATION: {}",
                        open_error("relation", &relations_path, &e)
                    )
                }),
            TableOpen::Opened(relation_store) => {
                let adapter =
                    RelationConnectionStore::new(GenericProductionStore::new(relation_store));
                if mvcc_enabled {
                    adapter.with_mvcc(&relations_path).unwrap_or_else(|e| {
                        panic!("SERVER_MVCC_ISOLATION: activating for relation: {e}")
                    })
                } else {
                    adapter
                }
            }
        };
        // `BAK-FR-002` (ADR-0065): only a durable table has a directory
        // worth naming — a scratch table is gone on restart regardless.
        let adapter = if durable {
            adapter.with_backup_source(relations_path)
        } else {
            adapter
        };
        // `SYU-FR-004` (ADR-0097): `msync` before every in-place
        // update's acknowledgement, when asked.
        adapter
            .with_synced_updates(sync_updates)
            .with_mvcc_reclaim_every(mvcc_reclaim_every)
            .with_journaled_updates(journal_updates)
    });
    let entity_connection_store: Arc<dyn ConnectionStore> = Arc::new({
        let adapter = match entity_store {
            TableOpen::ReopenWithMvcc => EntityConnectionStore::open_with_mvcc(&entities_path)
                .unwrap_or_else(|e| {
                    panic!(
                        "SERVER_MVCC_ISOLATION: {}",
                        open_error("entity", &entities_path, &e)
                    )
                }),
            TableOpen::Opened(entity_store) => {
                let adapter = EntityConnectionStore::new(GenericProductionStore::new(entity_store));
                if mvcc_enabled {
                    adapter.with_mvcc(&entities_path).unwrap_or_else(|e| {
                        panic!("SERVER_MVCC_ISOLATION: activating for entity: {e}")
                    })
                } else {
                    adapter
                }
            }
        };
        let adapter = if durable {
            adapter.with_backup_source(entities_path)
        } else {
            adapter
        };
        // `SYU-FR-004` (ADR-0097): `msync` before every in-place
        // update's acknowledgement, when asked.
        adapter
            .with_synced_updates(sync_updates)
            .with_mvcc_reclaim_every(mvcc_reclaim_every)
            .with_journaled_updates(journal_updates)
    });
    // `SERVER_TXN_JOURNAL_PATH` (ADR-0025): with it, every transaction
    // batch is crash-atomic — journaled and fsync'd before its first
    // write, replayed on the next start. Set it the same way every start:
    // opening without it after a crash forgoes the replay. With
    // `SERVER_MVCC_ISOLATION` too, a reopened table takes the journaled
    // MVCC reopen (`JMC-FR-003`, ADR-0114).
    let journal_path = std::env::var_os("SERVER_TXN_JOURNAL_PATH").map(PathBuf::from);
    let connection_store = Arc::new({
        let adapter = match store {
            TableOpen::ReopenWithMvcc => match &journal_path {
                Some(journal_path) => {
                    MemoryConnectionStore::open_with_mvcc_journaled(&memories_path, journal_path)
                        .unwrap_or_else(|e| {
                            panic!(
                                "SERVER_MVCC_ISOLATION with SERVER_TXN_JOURNAL_PATH: {}",
                                open_error("memory", &memories_path, &e)
                            )
                        })
                }
                None => MemoryConnectionStore::open_with_mvcc(&memories_path).unwrap_or_else(|e| {
                    panic!(
                        "SERVER_MVCC_ISOLATION: {}",
                        open_error("memory", &memories_path, &e)
                    )
                }),
            },
            TableOpen::Opened(store) => {
                let adapter = match &journal_path {
                    Some(journal_path) => MemoryConnectionStore::with_journal(
                        GenericProductionStore::new(store),
                        journal_path,
                    )
                    .unwrap_or_else(|e| {
                        panic!("SERVER_TXN_JOURNAL_PATH configured but invalid: {e}")
                    }),
                    None => MemoryConnectionStore::new(GenericProductionStore::new(store)),
                };
                if mvcc_enabled {
                    adapter.with_mvcc(&memories_path).unwrap_or_else(|e| {
                        panic!("SERVER_MVCC_ISOLATION: activating for memory: {e}")
                    })
                } else {
                    adapter
                }
            }
        };
        let adapter = if durable {
            adapter.with_backup_source(memories_path)
        } else {
            adapter
        };
        // `SYU-FR-004` (ADR-0097): `msync` before every in-place
        // update's acknowledgement, when asked.
        adapter
            .with_synced_updates(sync_updates)
            .with_mvcc_reclaim_every(mvcc_reclaim_every)
            .with_journaled_updates(journal_updates)
    });

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
    // `EXP-FR-002`/`003` (ADR-0094): a non-loopback bind without both
    // authentication and TLS is refused at startup; `SERVER_ALLOW_INSECURE=1`
    // turns the refusal into a warning for the operator who means it.
    if let Err(exposure) = check_exposure(&addr, &options) {
        if allow_insecure_from_env() {
            eprintln!(
                "WARNING: listening on {addr} although {exposure} (SERVER_ALLOW_INSECURE=1 is set)"
            );
        } else {
            panic!(
                "refusing to listen on {addr}: {exposure}; bind a loopback address, configure \
                 what is missing, or set SERVER_ALLOW_INSECURE=1 to serve anyway (ADR-0094)"
            );
        }
    }
    // `SERVER_BACKUP_ROOT` (ADR-0065, `BAK-FR-003`): opt-in — unset,
    // every `Request::Backup` answers `Unsupported` server-wide, no new
    // filesystem-write surface at all.
    let options = match std::env::var_os("SERVER_BACKUP_ROOT") {
        Some(root) => options.with_backup_root(PathBuf::from(root)),
        None => options,
    };
    let backup_rooted = std::env::var_os("SERVER_BACKUP_ROOT").is_some();
    // `SERVER_AUTH_REPLICATION_TOKEN` (ADR-0067, `RPL-FR-002`): opt-in —
    // unset, every `Request::FetchSnapshot` answers `Unauthorized`
    // server-wide, no new network-egress surface at all.
    let options = match std::env::var("SERVER_AUTH_REPLICATION_TOKEN") {
        Ok(token) => options.with_replication_token(token),
        Err(_) => options,
    };
    let replication_tokened = std::env::var_os("SERVER_AUTH_REPLICATION_TOKEN").is_some();
    // `SERVER_IDLE_TIMEOUT_SECS` / `SERVER_MAX_CONNECTIONS` /
    // `SERVER_MAX_QUERY_ROWS` (ADR-0093, `LIM-FR-005`): each opt-in; a
    // value that is not a positive integer is a startup error, this
    // binary's convention for every other misconfiguration.
    // Defaults since `ADR-0099` (`DEF-FR-001`): 300 s idle, 1,024
    // connections; since `ADR-0102` (`CLP-FR-003`) 10,000 rows, now that a
    // `Query` with no `limit` is clamped under the cap rather than refused.
    let options = match bounded_env("SERVER_IDLE_TIMEOUT_SECS", Some(DEFAULT_IDLE_TIMEOUT_SECS)) {
        Some(secs) => options.with_idle_timeout(std::time::Duration::from_secs(secs)),
        None => options,
    };
    let options = match bounded_env("SERVER_MAX_CONNECTIONS", Some(DEFAULT_MAX_CONNECTIONS)) {
        Some(max) => options.with_max_connections(usize::try_from(max).unwrap_or(usize::MAX)),
        None => options,
    };
    let options = match bounded_env("SERVER_MAX_QUERY_ROWS", Some(DEFAULT_MAX_QUERY_ROWS)) {
        Some(max) => options.with_max_query_rows(usize::try_from(max).unwrap_or(usize::MAX)),
        None => options,
    };
    // `SERVER_METRICS_HTTP_ADDR` (ADR-0069, `MHTTP-FR-001`/`006`):
    // a separate, opt-in scrape listener; a bind failure is fatal at startup.
    let options = match std::env::var("SERVER_METRICS_HTTP_ADDR") {
        Ok(addr) => {
            // `RVM-FR-004` (ADR-0111): the same rule as the wire listener,
            // stricter — this one has no auth or TLS to configure.
            if let Err(exposure) = check_metrics_exposure(&addr) {
                if allow_insecure_from_env() {
                    eprintln!(
                        "WARNING: metrics listening on {addr} although {exposure} (SERVER_ALLOW_INSECURE=1 is set)"
                    );
                } else {
                    panic!(
                        "refusing to listen on {addr}: {exposure}; bind the metrics listener to a \
                         loopback address, or set SERVER_ALLOW_INSECURE=1 to serve anyway (ADR-0111)"
                    );
                }
            }
            let listener = TcpListener::bind(&addr)
                .expect("binding SERVER_METRICS_HTTP_ADDR for the HTTP metrics listener");
            eprintln!("memory_server metrics HTTP listening on {addr} (SERVER_METRICS_HTTP_ADDR, ADR-0069)");
            options.with_metrics_http(listener)
        }
        Err(_) => options,
    };
    eprintln!(
        "memory_server listening on {addr} (data: {}, auth: {}, TLS: {}, transaction journal: {}, audit log: {}, auth rate limit: {}, access log: {}, backup root: {}, replication token: {}, idle timeout: {:?}, max connections: {:?}, max query rows: {:?}, synced updates: {}, MVCC reclaim every: {:?}, journaled updates: {} — see ADR-0012/ADR-0014/ADR-0023/ADR-0025/ADR-0029/ADR-0030/ADR-0031/ADR-0048/ADR-0053/ADR-0064/ADR-0065/ADR-0067; do not expose beyond a trusted network unless auth and TLS are both configured)",
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
        if backup_rooted { "configured" } else { "NOT configured" },
        if replication_tokened { "configured" } else { "NOT configured" },
        options.idle_timeout(),
        options.max_connections(),
        options.max_query_rows(),
        if sync_updates { "configured" } else { "NOT configured (write-back)" },
        mvcc_reclaim_every,
        if journal_updates { "configured" } else { "NOT configured" },
    );

    // `TBL-FR-001` (ADR-0050): tables on one listener, `memory` primary
    // — `Use entity` reaches the second; `JOIN entity e ON mentions`
    // crosses between them; `Use relation` (ADR-0058) reaches the third.
    let memory_connection_store: Arc<dyn ConnectionStore> = connection_store;
    serve_tables(
        listener,
        vec![
            ("memory".to_string(), memory_connection_store),
            ("entity".to_string(), entity_connection_store),
            ("relation".to_string(), relation_connection_store),
        ],
        0,
        options,
    );
}

/// Where this process keeps its three tables (`DDR-FR-002`).
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
/// How a table's store reaches its adapter (`JMC-FR-003`, ADR-0114):
/// opened here, or left to the adapter's own MVCC reopen, which reads the
/// insert log before any open of the file clears it.
enum TableOpen<S> {
    Opened(S),
    ReopenWithMvcc,
}

/// `SCE-FR-001` (ADR-0116): the startup error for a table that will not
/// open, and — when the cause is a schema-tag mismatch, the distinct
/// refusal `ADR-0056` gives a directory written at an older layout —
/// the remedy: the migration tool `ADR-0066` ships for `Memory`, or a
/// re-push from the consumer, the source of truth, for every table.
fn open_error(table: &str, path: &Path, error: &dyn std::fmt::Display) -> String {
    let text = error.to_string();
    let mut message = format!(
        "SERVER_DATA_DIR: opening {} ({table}): {text}",
        path.display()
    );
    if text.contains("schema tag mismatch") {
        message.push_str(
            "\nThis directory was written at an older layout of this table than this binary \
             serves (ADR-0056). It is refused, never mis-read and never recreated.",
        );
        if table == "memory" {
            // `RGF-FR-003` (ADR-0120): the tool takes the two *slot file*
            // paths and copies only this table, so the remedy names the
            // directory `SERVER_DATA_DIR` must point at and the two
            // sibling tables the operator carries over unchanged.
            message.push_str(
                "\nRemedy: migrate it into a fresh directory <new_dir>:\n  cargo run -p \
                 rusty_multimodal_db --example migrate_memory_v1_to_v2 -- \
                 <old_dir>/memories.mmap <new_dir>/memories.mmap\n(ADR-0066, STORAGE-019; \
                 <new_dir>/memories.mmap must not exist yet). The tool migrates the memory \
                 table only: copy every entities.mmap* and relations.mmap* file from <old_dir> \
                 into <new_dir> unchanged (their layouts have not changed), then set \
                 SERVER_DATA_DIR=<new_dir>. Or re-push the table from the consumer, the source \
                 of truth.",
            );
        } else {
            message.push_str(
                "\nRemedy: re-push the table from the consumer, the source of truth (no \
                 migration tool ships for this table yet — ADR-0066's three-step pattern \
                 applies when one is needed).",
            );
        }
    }
    message
}

/// The three tables as `open_stores` leaves them, with their file paths.
struct OpenedStores {
    store: TableOpen<MemoryProductionStack>,
    entity_store: TableOpen<EntityProductionStack>,
    relation_store: TableOpen<RelationProductionStack>,
    memories_path: PathBuf,
    entities_path: PathBuf,
    relations_path: PathBuf,
}

fn open_stores(data: &DataLocation, mvcc_enabled: bool) -> Result<OpenedStores, String> {
    let (dir, durable) = match data {
        DataLocation::Durable(dir) => (dir, true),
        DataLocation::Scratch(dir) => (dir, false),
    };
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("creating the data directory {}: {e}", dir.display()))?;
    let memories = dir.join("memories.mmap");
    let entities = dir.join("entities.mmap");
    let relations = dir.join("relations.mmap");
    if durable {
        // Existence is checked before any open: afterward every file
        // exists regardless. Scratch mode is always fresh — its directory
        // is a brand-new per-process temp path.
        let store = if mvcc_enabled && memories.exists() {
            TableOpen::ReopenWithMvcc
        } else {
            TableOpen::Opened(
                open_or_create_memory_production_stack(&memories)
                    .map_err(|e| open_error("memory", &memories, &e))?,
            )
        };
        let entity_store = if mvcc_enabled && entities.exists() {
            TableOpen::ReopenWithMvcc
        } else {
            TableOpen::Opened(
                open_or_create_entity_production_stack(&entities)
                    .map_err(|e| open_error("entity", &entities, &e))?,
            )
        };
        let relation_store = if mvcc_enabled && relations.exists() {
            TableOpen::ReopenWithMvcc
        } else {
            TableOpen::Opened(
                open_or_create_relation_production_stack(&relations)
                    .map_err(|e| open_error("relation", &relations, &e))?,
            )
        };
        return Ok(OpenedStores {
            store,
            entity_store,
            relation_store,
            memories_path: memories,
            entities_path: entities,
            relations_path: relations,
        });
    }
    let store = create_memory_production_stack(sample_memories(), &sample_mentions(), &memories)
        .map_err(|e| format!("creating the sample MemoryProductionStack: {e}"))?;
    let entity_store = create_entity_production_stack(sample_entities(), &[], &[], &entities)
        .map_err(|e| format!("creating the sample EntityProductionStack: {e}"))?;
    let relation_store = create_relation_production_stack(sample_relations(), &relations)
        .map_err(|e| format!("creating the sample RelationProductionStack: {e}"))?;
    Ok(OpenedStores {
        store: TableOpen::Opened(store),
        entity_store: TableOpen::Opened(entity_store),
        relation_store: TableOpen::Opened(relation_store),
        memories_path: memories,
        entities_path: entities,
        relations_path: relations,
    })
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

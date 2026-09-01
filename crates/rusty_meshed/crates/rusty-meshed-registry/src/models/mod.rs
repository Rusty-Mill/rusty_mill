//! Table models and DB schema for the data product registry -- the
//! Rust port of `meshed.registry.models` (REG-012..025, REG-138) and
//! `meshed.registry.schemas` (see the `schemas` submodule for
//! REG-026..033, REG-139).
//!
//! Unlike the Python source, which keeps SQLModel table models
//! entirely separate from the Pydantic Create/Public/Update schemas
//! (to avoid ever leaking a lazy-load-capable ORM object out of a
//! route), the structs here double as both: there is no lazy loading
//! to leak, so [`DataProduct`]/[`InputPort`]/[`OutputPort`] are already
//! the "public" shape. The one place the two representations
//! genuinely diverge is [`DataContract`] vs. `schemas::DataContractPublic`
//! -- `quality_assertions` is JSON-encoded in storage but decoded to a
//! `Vec<String>` in the public view (REG-031).

pub mod enums;
pub mod schemas;

pub use enums::MaturityTier;

use rusty_meshed_core::EventType;
use rusty_sqlite::rusqlite::{Connection, Result as SqlResult};

/// A persisted data product record -- the authoritative registry entry
/// (`meshed.registry.models.DataProduct`). All ports and contracts are
/// owned by this record via `ON DELETE CASCADE` FKs (REG-018, REG-019).
///
/// `tags` is returned to callers exactly as stored -- a raw
/// JSON-encoded string, never decoded (REG-138), unlike
/// [`DataContract`]'s `quality_assertions` in its public view.
#[derive(Debug, Clone, PartialEq)]
pub struct DataProduct {
    pub id: i64,
    pub name: String,
    pub owner: String,
    pub version: String,
    pub domain: String,
    pub description: String,
    pub maturity_tier: MaturityTier,
    pub tags: String,
}

/// An input port consumed by a data product -- the upstream Kafka
/// topics a data product reads from.
#[derive(Debug, Clone, PartialEq)]
pub struct InputPort {
    pub id: i64,
    pub data_product_id: i64,
    pub topic_name: String,
    pub description: Option<String>,
}

/// An output port exposed by a data product -- the Kafka topics a
/// product publishes, with schema subject and event classification.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputPort {
    pub id: i64,
    pub data_product_id: i64,
    pub topic_name: String,
    pub schema_subject: String,
    pub event_type: EventType,
    pub description: Option<String>,
}

/// A data contract for an output port -- schema reference, SLOs, and
/// quality assertions consumers can rely on. Exactly one contract per
/// port at the DB schema level (`output_port_id` is `UNIQUE`,
/// REG-023).
///
/// `quality_assertions` is the raw JSON-encoded storage form; see
/// [`schemas::DataContractPublic::from_row`] for the decoded public
/// view (REG-031).
#[derive(Debug, Clone, PartialEq)]
pub struct DataContract {
    pub id: i64,
    pub output_port_id: i64,
    pub schema_ref: String,
    pub owner: String,
    pub slo_freshness_seconds: i64,
    pub slo_completeness_pct: f64,
    pub quality_assertions: String,
}

/// A grant authorizing a consumer group to resolve (and thus subscribe
/// to) a specific output port's topic -- the Rust port of
/// `meshed.governance.rbac.PortAccessGrant` (GOV-013). Lives in this
/// crate rather than `rusty-meshed-governance` despite the Python
/// module's name (`rbac.py`) -- it's a SQLite-backed table + HTTP
/// routes, not part of the governance engine itself (see that crate's
/// module doc for the same note). Unlike [`InputPort`]/[`OutputPort`],
/// `output_port_id` has no `ON DELETE CASCADE`, matching the source's
/// plain `foreign_key="output_ports.id"` with no cascade declared, and
/// there's no DB-level uniqueness on `(output_port_id,
/// consumer_group_id)` either (REG-091) -- duplicates are rejected
/// only at the API layer (409, GOV-015).
#[derive(Debug, Clone, PartialEq)]
pub struct PortAccessGrant {
    pub id: i64,
    pub output_port_id: i64,
    pub consumer_group_id: String,
    pub granted_by: String,
    pub granted_at: String,
}

/// Creates the five registry tables if they don't already exist, and
/// turns on FK enforcement (SQLite disables it by default per
/// connection) so the `ON DELETE CASCADE` clauses below actually fire
/// -- matching `SQLModel.metadata.create_all`'s no-migrations approach
/// plus the source's `sa_relationship_kwargs={"cascade": "all,
/// delete-orphan"}` (REG-018, REG-019, REG-020) at the DB-schema
/// level. Must be called on every connection that needs cascade
/// deletes to work, since `PRAGMA foreign_keys` is per-connection
/// state, not persisted schema.
pub fn ensure_schema(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS data_products (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            owner TEXT NOT NULL,
            version TEXT NOT NULL,
            domain TEXT NOT NULL,
            description TEXT NOT NULL,
            maturity_tier TEXT NOT NULL DEFAULT 'mvp',
            tags TEXT NOT NULL DEFAULT '[]'
        );
        CREATE INDEX IF NOT EXISTS idx_data_products_name ON data_products(name);
        CREATE INDEX IF NOT EXISTS idx_data_products_domain ON data_products(domain);

        CREATE TABLE IF NOT EXISTS input_ports (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            data_product_id INTEGER NOT NULL REFERENCES data_products(id) ON DELETE CASCADE,
            topic_name TEXT NOT NULL,
            description TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_input_ports_data_product_id ON input_ports(data_product_id);

        CREATE TABLE IF NOT EXISTS output_ports (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            data_product_id INTEGER NOT NULL REFERENCES data_products(id) ON DELETE CASCADE,
            topic_name TEXT NOT NULL,
            schema_subject TEXT NOT NULL,
            event_type TEXT NOT NULL,
            description TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_output_ports_data_product_id ON output_ports(data_product_id);

        CREATE TABLE IF NOT EXISTS data_contracts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            output_port_id INTEGER NOT NULL UNIQUE REFERENCES output_ports(id) ON DELETE CASCADE,
            schema_ref TEXT NOT NULL,
            owner TEXT NOT NULL,
            slo_freshness_seconds INTEGER NOT NULL,
            slo_completeness_pct REAL NOT NULL,
            quality_assertions TEXT NOT NULL DEFAULT '[]'
        );

        CREATE TABLE IF NOT EXISTS port_access_grants (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            output_port_id INTEGER NOT NULL REFERENCES output_ports(id),
            consumer_group_id TEXT NOT NULL,
            granted_by TEXT NOT NULL,
            granted_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_port_access_grants_output_port_id ON port_access_grants(output_port_id);
        CREATE INDEX IF NOT EXISTS idx_port_access_grants_consumer_group_id ON port_access_grants(consumer_group_id);",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_sqlite::rusqlite::params;

    fn seeded_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        conn
    }

    fn insert_product(conn: &Connection) -> i64 {
        conn.execute(
            "INSERT INTO data_products (name, owner, version, domain, description) VALUES (?1, ?2, ?3, ?4, ?5)",
            params!["orders", "team-a", "1.0.0", "commerce", "Order events"],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn ensure_schema_is_idempotent() {
        let conn = seeded_connection();
        ensure_schema(&conn).unwrap();
        ensure_schema(&conn).unwrap();
    }

    #[test]
    fn data_product_maturity_tier_and_tags_default_at_the_schema_level() {
        let conn = seeded_connection();
        let id = insert_product(&conn);
        let (tier, tags): (String, String) = conn
            .query_row(
                "SELECT maturity_tier, tags FROM data_products WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(tier, "mvp");
        assert_eq!(tags, "[]");
    }

    #[test]
    fn deleting_a_data_product_cascades_to_its_input_ports() {
        let conn = seeded_connection();
        let product_id = insert_product(&conn);
        conn.execute(
            "INSERT INTO input_ports (data_product_id, topic_name) VALUES (?1, ?2)",
            params![product_id, "upstream.topic"],
        )
        .unwrap();

        conn.execute(
            "DELETE FROM data_products WHERE id = ?1",
            params![product_id],
        )
        .unwrap();

        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM input_ports", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn deleting_a_data_product_cascades_to_its_output_ports_and_their_contracts() {
        let conn = seeded_connection();
        let product_id = insert_product(&conn);
        conn.execute(
            "INSERT INTO output_ports (data_product_id, topic_name, schema_subject, event_type) VALUES (?1, ?2, ?3, ?4)",
            params![product_id, "downstream.topic", "downstream.topic-value", "delta"],
        )
        .unwrap();
        let port_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO data_contracts (output_port_id, schema_ref, owner, slo_freshness_seconds, slo_completeness_pct)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![port_id, "downstream.topic-value:1", "team-a", 60, 99.5],
        )
        .unwrap();

        conn.execute(
            "DELETE FROM data_products WHERE id = ?1",
            params![product_id],
        )
        .unwrap();

        let ports: i64 = conn
            .query_row("SELECT COUNT(*) FROM output_ports", [], |row| row.get(0))
            .unwrap();
        let contracts: i64 = conn
            .query_row("SELECT COUNT(*) FROM data_contracts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(ports, 0);
        assert_eq!(
            contracts, 0,
            "deleting the product's output port must cascade to its contract too"
        );
    }

    #[test]
    fn an_output_port_may_have_at_most_one_data_contract() {
        let conn = seeded_connection();
        let product_id = insert_product(&conn);
        conn.execute(
            "INSERT INTO output_ports (data_product_id, topic_name, schema_subject, event_type) VALUES (?1, ?2, ?3, ?4)",
            params![product_id, "downstream.topic", "downstream.topic-value", "delta"],
        )
        .unwrap();
        let port_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO data_contracts (output_port_id, schema_ref, owner, slo_freshness_seconds, slo_completeness_pct)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![port_id, "downstream.topic-value:1", "team-a", 60, 99.5],
        )
        .unwrap();

        let second_insert = conn.execute(
            "INSERT INTO data_contracts (output_port_id, schema_ref, owner, slo_freshness_seconds, slo_completeness_pct)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![port_id, "downstream.topic-value:2", "team-a", 60, 99.5],
        );
        assert!(
            second_insert.is_err(),
            "output_port_id must be UNIQUE on data_contracts"
        );
    }

    #[test]
    fn an_input_port_requires_a_valid_data_product_fk() {
        let conn = seeded_connection();
        let result = conn.execute(
            "INSERT INTO input_ports (data_product_id, topic_name) VALUES (?1, ?2)",
            params![999_999, "orphaned.topic"],
        );
        assert!(
            result.is_err(),
            "data_product_id must reference an existing data_products row"
        );
    }

    #[test]
    fn port_access_grants_has_no_composite_uniqueness_at_the_db_level() {
        let conn = seeded_connection();
        let product_id = insert_product(&conn);
        conn.execute(
            "INSERT INTO output_ports (data_product_id, topic_name, schema_subject, event_type) VALUES (?1, ?2, ?3, ?4)",
            params![product_id, "downstream.topic", "downstream.topic-value", "delta"],
        )
        .unwrap();
        let port_id = conn.last_insert_rowid();

        for _ in 0..2 {
            conn.execute(
                "INSERT INTO port_access_grants (output_port_id, consumer_group_id, granted_by, granted_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![port_id, "billing-service", "admin@example.com", "2026-01-01T00:00:00Z"],
            )
            .unwrap();
        }

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM port_access_grants", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            count, 2,
            "REG-091: no DB-level composite uniqueness on (output_port_id, consumer_group_id)"
        );
    }
}

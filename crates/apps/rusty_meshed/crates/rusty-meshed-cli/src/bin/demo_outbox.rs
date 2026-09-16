//! `demo_outbox` -- the Rust port of `scripts/demo_outbox.py`
//! (CLI-043..045): demonstrates the transactional outbox pattern's
//! core invariant, that a business write and its outbox entry commit
//! in one atomic transaction, then relays the pending entry to Kafka
//! via a background [`OutboxRelay`](rusty_meshed_sdk::OutboxRelay).
//!
//! Environment variables (matching the source exactly):
//! - `MESHED_COMPOSE_UP`: any non-empty value enables the Kafka relay
//!   step (CLI-043).
//! - `DEMO_DB_PATH`: SQLite database path, default `"demo_outbox.db"`.
//! - `KAFKA_BOOTSTRAP_SERVERS`: default `"localhost:9092"`.
//!
//! With `MESHED_COMPOSE_UP` unset, Step 2 skips the relay, prints every
//! outbox row, exits 0 (CLI-044). With it set, Step 2 starts an
//! [`OutboxRelay`](rusty_meshed_sdk::OutboxRelay) background thread
//! and polls every 0.25s for up to 10s for the entry's `published_at`
//! to be set (CLI-045); success prints the full entry, a timeout
//! prints an error and exits 1. `OutboxRelay` itself landed in an
//! earlier pass (SDK-057..064, once `rusty_kafka`'s `Produce` support
//! existed) -- this binary just hadn't been wired up to actually use
//! it yet.

use rusty_json::json;
use rusty_meshed_sdk::outbox;
use rusty_meshed_sdk::OutboxRelay;
use rusty_sqlite::rusqlite::Connection;
use std::time::{Duration, Instant};

const DEMO_TOPIC: &str = "meshed.demo.outbox-events";

/// The one column [`main`]'s poll loop actually needs -- checked every
/// 0.25s until it's `Some` or the 10s deadline passes (CLI-045). `id`
/// is always the row this binary just inserted itself in Step 1, so a
/// missing row is not a case worth distinguishing from a `NULL`
/// `published_at` -- both just mean "not published yet" here.
fn fetch_published_at(conn: &Connection, entry_id: i64) -> Option<String> {
    conn.query_row(
        "SELECT published_at FROM outbox_entries WHERE id = ?1",
        [entry_id],
        |row| row.get::<_, Option<String>>(0),
    )
    .ok()
    .flatten()
}

fn main() {
    let compose_up = std::env::var("MESHED_COMPOSE_UP").unwrap_or_default();
    let db_path = std::env::var("DEMO_DB_PATH").unwrap_or_else(|_| "demo_outbox.db".to_string());
    let bootstrap_servers =
        std::env::var("KAFKA_BOOTSTRAP_SERVERS").unwrap_or_else(|_| "localhost:9092".to_string());

    println!("{}", "=".repeat(60));
    println!("Transactional Outbox Pattern — End-to-End Demo");
    println!("{}", "=".repeat(60));
    println!("  Database : {db_path}");
    println!(
        "  Kafka    : {}",
        if compose_up.is_empty() {
            "disabled (MESHED_COMPOSE_UP not set)".to_string()
        } else {
            format!("enabled ({bootstrap_servers})")
        }
    );
    println!();

    let mut conn = Connection::open(&db_path).expect("failed to open outbox database");
    outbox::ensure_schema(&conn).expect("failed to create outbox schema");

    println!("Step 1: Writing business record + outbox entry in a single transaction...");
    let payload = json!({
        "unit_id": "ALPHA-1",
        "person_id": "SGT-42",
        "role": "Team Leader",
        "effective_date": "2026-03-11"
    });
    let headers = json!({"source": "demo_outbox", "version": "1"});

    let entry_id = {
        let tx = conn.transaction().expect("failed to start transaction");
        // In a real application you would also write your business
        // entity here, in this same transaction.
        let entry = outbox::write_outbox_entry(
            &tx,
            "PersonnelAssigned",
            DEMO_TOPIC,
            &payload,
            Some(&headers),
        )
        .expect("failed to write outbox entry");
        tx.commit().expect("failed to commit transaction"); // <-- single atomic commit for both writes
        entry.id
    };
    println!("  Committed. OutboxEntry id={entry_id}, published_at=None (pending)");
    println!();

    if compose_up.is_empty() {
        println!("Step 2: Skipping Kafka relay (MESHED_COMPOSE_UP not set).");
        println!();
        println!("To run the full end-to-end demo with Kafka:");
        println!("  MESHED_COMPOSE_UP=1 demo_outbox");
        println!();
        println!("Current outbox state:");
        let entries = outbox::fetch_all(&conn).expect("failed to read outbox entries");
        for entry in &entries {
            println!(
                "  id={} event_type={} topic={} published_at={}",
                entry.id,
                entry.event_type,
                entry.topic,
                entry.published_at.as_deref().unwrap_or("None")
            );
        }
        std::process::exit(0);
    }

    println!("Step 2: Starting OutboxRelay background thread...");
    let mut relay = OutboxRelay::new(db_path.clone(), bootstrap_servers.clone());
    relay.start();

    println!("  Waiting up to 10 seconds for entry id={entry_id} to be published...");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut published_at = None;
    while Instant::now() < deadline {
        if let Some(value) = fetch_published_at(&conn, entry_id) {
            published_at = Some(value);
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    relay.stop();

    println!();
    match published_at {
        Some(published_at) => {
            println!("Outbox entry relayed to Kafka topic {DEMO_TOPIC} at {published_at}");
            println!();
            println!("Entry details:");
            let entries = outbox::fetch_all(&conn).expect("failed to read outbox entries");
            let entry = entries
                .into_iter()
                .find(|e| e.id == entry_id)
                .expect("just-relayed entry must still exist");
            let pretty_payload = rusty_json::from_str::<rusty_json::Value>(&entry.payload)
                .ok()
                .and_then(|value| {
                    rusty_json::to_string_with_formatter(
                        &value,
                        rusty_json::PrettyFormatter::with_indent_width(4),
                    )
                    .ok()
                })
                .unwrap_or(entry.payload);
            println!("  id          : {}", entry.id);
            println!("  event_type  : {}", entry.event_type);
            println!("  topic       : {}", entry.topic);
            println!("  payload     : {pretty_payload}");
            println!("  created_at  : {}", entry.created_at);
            println!(
                "  published_at: {}",
                entry.published_at.as_deref().unwrap_or("None")
            );
        }
        None => {
            println!("ERROR: Entry id={entry_id} was not relayed within 10 seconds.");
            println!("Check that Kafka is accessible at: {bootstrap_servers}");
            std::process::exit(1);
        }
    }

    println!();
    println!("Demo complete.");
}

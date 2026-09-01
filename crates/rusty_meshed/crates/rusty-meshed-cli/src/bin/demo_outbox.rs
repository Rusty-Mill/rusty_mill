//! `demo_outbox` -- the Rust port of `scripts/demo_outbox.py`
//! (CLI-043..045): demonstrates the transactional outbox pattern's
//! core invariant, that a business write and its outbox entry commit
//! in one atomic transaction.
//!
//! Environment variables (matching the source exactly):
//! - `MESHED_COMPOSE_UP`: any non-empty value enables the Kafka relay
//!   step (CLI-043).
//! - `DEMO_DB_PATH`: SQLite database path, default `"demo_outbox.db"`.
//! - `KAFKA_BOOTSTRAP_SERVERS`: default `"localhost:9092"`.
//!
//! **Step 2 (the relay) is not fully ported.** With `MESHED_COMPOSE_UP`
//! unset, this matches the source exactly: skip the relay, print every
//! outbox row, exit 0 (CLI-044). With it set, the source starts an
//! `OutboxRelay` background thread and waits up to 10s for the entry
//! to publish (CLI-045) -- `rusty-meshed-sdk::outbox` doesn't implement
//! `OutboxRelay` at all (it needs a Kafka `Produce` request
//! `rusty_kafka` doesn't have, see that module's own doc and issue
//! #87), so this binary reports that plainly and exits 1 rather than
//! pretending to wait for a relay that doesn't exist.

use rusty_json::json;
use rusty_meshed_sdk::outbox;
use rusty_sqlite::rusqlite::Connection;

const DEMO_TOPIC: &str = "meshed.demo.outbox-events";

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

    println!("Step 2: Kafka relay requested (MESHED_COMPOSE_UP set).");
    println!(
        "  OutboxRelay isn't implemented in this build yet -- it needs a Kafka Produce request"
    );
    println!("  rusty_kafka doesn't have (see rusty-meshed-sdk::outbox's module doc).");
    println!();
    println!("ERROR: Entry id={entry_id} cannot be relayed (no relay implementation).");
    std::process::exit(1);
}

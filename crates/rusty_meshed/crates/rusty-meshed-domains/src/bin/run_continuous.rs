//! `run_continuous` -- the Rust port of
//! `meshed.domains.generators.run_continuous` (DOM-037..042): a
//! repeating randomized-scenario generator for live dashboard demos.
//! Each cycle builds a fresh [`ScenarioBuilder`] scenario with
//! randomized personnel/positions/event mix, publishes every event
//! with a short delay between them (for visual effect in a monitor),
//! then sleeps `SCENARIO_INTERVAL` before the next cycle. Runs until
//! interrupted (Ctrl+C).
//!
//! ```text
//! cargo run -p rusty-meshed-domains --bin run_continuous
//! ```
//!
//! Environment variables (DOM-037):
//!
//! - `KAFKA_BOOTSTRAP_SERVERS` (default `localhost:9092`)
//! - `SCHEMA_REGISTRY_URL` (default `http://localhost:8081`)
//! - `REGISTRY_API_URL` (default `http://localhost:8100` -- note this
//!   differs from `run_scenario`'s own `:8000` default, matching the
//!   source exactly)
//! - `SCENARIO_INTERVAL` (seconds between cycles, default `5`)
//!
//! # Randomization has no source-parity requirement on its own values
//!
//! `Rng` below is a small, dependency-free, non-cryptographic
//! splitmix64 PRNG seeded from the wall clock -- this workspace never
//! pulls an external `rand` crate (see e.g. `rusty_uuid`/`rusty_oauth`'s
//! own hand-rolled OS-CSPRNG readers for the same reason, though this
//! one need not be cryptographically secure: it only picks which demo
//! IDs/grades/duties to publish, not anything security-sensitive). The
//! exact random values produced obviously can't match the source's
//! own `random` module bit-for-bit; DOM-040's capability is the
//! generation *shape* (2-5 people, E4-E7 authorized grades, 60%/30%
//! promotion/separation odds, etc.), which this port reproduces
//! exactly.
//!
//! # No per-cycle `personnel_producer.flush()` call
//!
//! The source calls `personnel_producer.flush()` every cycle, which
//! resolves to the *inherited*, unoverridden base `flush()` --
//! `PersonnelLifecycleProducer` (Python) has no `flush()` override of
//! its own (its `publish()` override writes straight to the outbox,
//! synchronously, so there's nothing buffered to flush either). This
//! port's [`rusty_meshed_domains::products::PersonnelLifecycleProducer`]
//! doesn't expose a `flush()` method at all for the same reason --
//! there'd be nothing for it to do. `position_producer.flush()` *is*
//! called, matching the source, since [`DataProductProducerBase::flush`]
//! is a real (if documented no-op) method there.

use rusty_meshed_core::PlatformConfig;
use rusty_meshed_domains::generators::topics::{create_phase4_topics, event_topic, PHASE4_TOPICS};
use rusty_meshed_domains::generators::{ScenarioBuilder, ScenarioEvent};
use rusty_meshed_domains::products::{PersonnelLifecycleProducer, PositionManagementProducer};
use rusty_meshed_sdk::DataProductProducerBase;
use rusty_tokio::io::TcpStream;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const UNITS: [&str; 5] = [
    "UNIT-ALPHA",
    "UNIT-BRAVO",
    "UNIT-CHARLIE",
    "UNIT-DELTA",
    "UNIT-ECHO",
];
const GRADES: [&str; 9] = ["E1", "E2", "E3", "E4", "E5", "E6", "E7", "E8", "E9"];
const DUTIES: [&str; 12] = [
    "Rifleman",
    "Team Leader",
    "Squad Leader",
    "Platoon Sergeant",
    "Radio Operator",
    "Medic",
    "Driver",
    "Gunner",
    "Section Chief",
    "Intelligence Analyst",
    "Supply Sergeant",
    "Operations NCO",
];
const SEPARATION_REASONS: [&str; 5] = ["ETS", "RETIREMENT", "MEDICAL", "HARDSHIP", "PCS"];

/// A small, dependency-free, non-cryptographic splitmix64 PRNG -- see
/// the module doc for why.
struct Rng(u64);

impl Rng {
    fn new() -> Self {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniformly distributed value in `[low, high)`.
    fn gen_range(&mut self, low: u64, high: u64) -> u64 {
        low + self.next_u64() % (high - low)
    }

    /// `true` with probability `probability` (`0.0..=1.0`).
    fn gen_bool(&mut self, probability: f64) -> bool {
        (self.next_u64() as f64 / u64::MAX as f64) < probability
    }

    fn choice<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.gen_range(0, items.len() as u64) as usize]
    }
}

/// Builds a randomized scenario with 2-5 personnel and positions
/// (DOM-040): activates everyone, authorizes a matching position per
/// person (random grade E4-E7, random duty), assigns and fills every
/// pair, then a 60% chance of one promotion and a 30% chance of one
/// separation.
fn build_random_scenario(rng: &mut Rng) -> (String, Vec<ScenarioEvent>) {
    let mut builder = ScenarioBuilder::new();
    let num_people = rng.gen_range(2, 6) as usize;
    let unit = *rng.choice(&UNITS);

    let person_ids: Vec<String> = (0..num_people)
        .map(|_| format!("P{}", rng.gen_range(1000, 10000)))
        .collect();
    let position_ids: Vec<String> = (0..num_people)
        .map(|_| format!("POS{}", rng.gen_range(1000, 10000)))
        .collect();

    for person_id in &person_ids {
        builder = builder.add_status_change(person_id, "ACTIVE", "NONE", 0);
    }

    let grades: Vec<&str> = (0..num_people)
        .map(|_| *rng.choice(&GRADES[3..7]))
        .collect();
    let duties: Vec<&str> = (0..num_people).map(|_| *rng.choice(&DUTIES)).collect();
    for i in 0..num_people {
        builder =
            builder.add_position_authorization(&position_ids[i], unit, grades[i], duties[i], 0);
    }

    for i in 0..num_people {
        builder = builder
            .add_assignment(
                &person_ids[i],
                &position_ids[i],
                unit,
                duties[i],
                grades[i],
                0,
            )
            .expect("person was just activated above");
        builder = builder
            .add_position_fill(&position_ids[i], &person_ids[i], unit, 0)
            .expect("position was just authorized above");
    }

    if rng.gen_bool(0.6) && num_people >= 2 {
        let promoted = rng.choice(&person_ids).clone();
        let from_grade = *rng.choice(&GRADES[3..6]);
        let from_idx = GRADES.iter().position(|&g| g == from_grade).unwrap();
        let to_grade = GRADES.get(from_idx + 1).copied().unwrap_or(from_grade);
        builder = builder.add_promotion(promoted, from_grade, to_grade, 1);
    }

    if rng.gen_bool(0.3) {
        let separated = rng.choice(&person_ids).clone();
        let reason = *rng.choice(&SEPARATION_REASONS);
        builder = builder.add_separation(separated, reason, 1);
    }

    let correlation_id = builder.correlation_id().to_string();
    (correlation_id, builder.build())
}

/// Publishes every event to its mapped topic (readiness events have no
/// mapping and are skipped, DOM-042), sleeping a random 0.1-0.4s
/// between each (visual effect for a live monitor). Returns how many
/// events were published.
async fn publish_scenario(
    events: &[ScenarioEvent],
    personnel_producer: &mut PersonnelLifecycleProducer<TcpStream>,
    position_producer: &mut DataProductProducerBase<TcpStream>,
    rng: &mut Rng,
) -> u64 {
    let mut published = 0u64;
    for event in events {
        let Some(topic) = event_topic(event.event_name()) else {
            continue;
        };
        match event {
            ScenarioEvent::StatusChanged(e) => personnel_producer
                .publish(topic, e)
                .expect("publish failed"),
            ScenarioEvent::PersonnelAssigned(e) => personnel_producer
                .publish(topic, e)
                .expect("publish failed"),
            ScenarioEvent::PersonnelPromoted(e) => personnel_producer
                .publish(topic, e)
                .expect("publish failed"),
            ScenarioEvent::PersonnelSeparated(e) => personnel_producer
                .publish(topic, e)
                .expect("publish failed"),
            ScenarioEvent::PositionAuthorizationChanged(e) => position_producer
                .publish(topic, e)
                .await
                .expect("publish failed"),
            ScenarioEvent::PositionFilled(e) => position_producer
                .publish(topic, e)
                .await
                .expect("publish failed"),
        }
        published += 1;

        let delay = 0.1 + (rng.next_u64() as f64 / u64::MAX as f64) * 0.3;
        rusty_tokio::time::sleep(Duration::from_secs_f64(delay)).await;
    }
    position_producer.flush(10.0);
    published
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

#[rusty_tokio::main]
async fn main() {
    let kafka_bootstrap = env_or("KAFKA_BOOTSTRAP_SERVERS", "localhost:9092");
    let schema_registry_url = env_or("SCHEMA_REGISTRY_URL", "http://localhost:8081");
    let registry_api_url = env_or("REGISTRY_API_URL", "http://localhost:8100");
    let scenario_interval: f64 = std::env::var("SCENARIO_INTERVAL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5.0);

    println!("=== Meshed Continuous Scenario Runner ===");
    println!("Kafka:           {kafka_bootstrap}");
    println!("Schema Registry: {schema_registry_url}");
    println!("Registry API:    {registry_api_url}");
    println!("Interval:        {scenario_interval:.1}s between scenarios");

    let mut admin_client = match rusty_kafka::KafkaClient::connect(
        &kafka_bootstrap,
        Some("run_continuous".to_string()),
    )
    .await
    {
        Ok(client) => client,
        Err(err) => {
            eprintln!("Cannot reach Kafka at {kafka_bootstrap}: {err}");
            std::process::exit(1);
        }
    };
    match create_phase4_topics(&mut admin_client).await {
        Ok(warnings) => {
            if warnings.is_empty() {
                println!("All {} Phase-4 topics ready.", PHASE4_TOPICS.len());
            }
            for warning in &warnings {
                eprintln!(
                    "Could not create topic {}: broker error code {}",
                    warning.topic, warning.error_code
                );
            }
        }
        Err(err) => {
            eprintln!("Cannot reach Kafka at {kafka_bootstrap}: {err}");
            std::process::exit(1);
        }
    }

    let config = PlatformConfig {
        kafka_bootstrap_servers: kafka_bootstrap.clone(),
        schema_registry_url: schema_registry_url.clone(),
        registry_base_url: registry_api_url.clone(),
        ..Default::default()
    };

    let mut personnel_producer = match PersonnelLifecycleProducer::connect(&config, None).await {
        Ok(producer) => producer,
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    };
    let mut position_producer = match DataProductProducerBase::connect(
        PositionManagementProducer::PRODUCT_NAME,
        PositionManagementProducer::DOMAIN,
        PositionManagementProducer::VERSION,
        PositionManagementProducer::OWNER,
        PositionManagementProducer::DESCRIPTION,
        PositionManagementProducer::output_ports(),
        &config,
    )
    .await
    {
        Ok(producer) => producer,
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    };

    if let Err(err) = personnel_producer.startup().await {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
    if let Err(err) = position_producer.startup().await {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }

    println!("Starting continuous event generation (Ctrl+C to stop)...");

    let mut rng = Rng::new();
    let mut cycle: u64 = 0;
    let mut total_published: u64 = 0;

    loop {
        // `return` from inside a `select!` arm returns from that arm's
        // own macro-generated closure, not from `main()` -- each arm
        // instead evaluates to `interrupted: bool`, checked (and acted
        // on) *outside* the macro.
        let interrupted = rusty_tokio::select! {
            (correlation_id, events) = async {
                let (correlation_id, events) = build_random_scenario(&mut rng);
                let published = publish_scenario(&events, &mut personnel_producer, &mut position_producer, &mut rng).await;
                (correlation_id, published)
            } => {
                cycle += 1;
                total_published += events;
                let short_id = &correlation_id[..correlation_id.len().min(8)];
                println!(
                    "Cycle {cycle}: {events} events (correlation={short_id}…) | Total: {total_published}"
                );
                false
            },
            _ = rusty_tokio::signal::ctrl_c() => true,
        };
        if interrupted {
            println!("Shutting down — {total_published} events published across {cycle} cycles.");
            return;
        }

        let interrupted = rusty_tokio::select! {
            _ = rusty_tokio::time::sleep(Duration::from_secs_f64(scenario_interval)) => false,
            _ = rusty_tokio::signal::ctrl_c() => true,
        };
        if interrupted {
            println!("Shutting down — {total_published} events published across {cycle} cycles.");
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_random_scenario_produces_2_to_5_people_worth_of_events() {
        let mut rng = Rng::new();
        let (_correlation_id, events) = build_random_scenario(&mut rng);
        // Every person contributes exactly 4 events (status change,
        // authorization, assignment, fill); num_people is 2..=5, so
        // the baseline is 8..=20, plus 0-2 optional promotion/separation.
        assert!(events.len() >= 8 && events.len() <= 22);
    }

    #[test]
    fn build_random_scenario_events_all_share_one_correlation_id() {
        let mut rng = Rng::new();
        let (correlation_id, events) = build_random_scenario(&mut rng);
        for event in &events {
            assert_eq!(event.base().correlation_id, correlation_id);
        }
    }

    #[test]
    fn rng_gen_range_stays_within_bounds() {
        let mut rng = Rng::new();
        for _ in 0..1000 {
            let value = rng.gen_range(2, 6);
            assert!((2..6).contains(&value));
        }
    }

    #[test]
    fn rng_choice_always_returns_an_element_of_the_slice() {
        let mut rng = Rng::new();
        for _ in 0..100 {
            let chosen = *rng.choice(&UNITS);
            assert!(UNITS.contains(&chosen));
        }
    }
}

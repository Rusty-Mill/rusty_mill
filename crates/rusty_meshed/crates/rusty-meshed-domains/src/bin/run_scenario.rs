//! `run_scenario` -- the Rust port of
//! `meshed.domains.generators.run_scenario` (DOM-043..047): an
//! end-to-end, one-shot demonstration of the cross-domain manpower
//! event flow. Creates the 9 Phase-4 topics, builds a fixed 14-event
//! demo scenario (3 people, 3 positions, one promotion, one
//! retroactive correction), publishes every event, and prints a
//! per-topic summary.
//!
//! ```text
//! cargo run -p rusty-meshed-domains --bin run_scenario
//! ```
//!
//! **Not run in CI** -- a manual demo requiring a live Kafka + Schema
//! Registry + Registry API stack, same as the source (DOM-043).
//!
//! Environment variables (DOM-043):
//!
//! - `KAFKA_BOOTSTRAP_SERVERS` (default `localhost:9092`)
//! - `SCHEMA_REGISTRY_URL` (default `http://localhost:8081`)
//! - `REGISTRY_API_URL` (default `http://localhost:8000` -- note this
//!   differs from `run_continuous`'s own `:8100` default, matching the
//!   source exactly)

use rusty_meshed_core::PlatformConfig;
use rusty_meshed_domains::generators::topics::{create_phase4_topics, event_topic, PHASE4_TOPICS};
use rusty_meshed_domains::generators::{ScenarioBuilder, ScenarioEvent};
use rusty_meshed_domains::products::{PersonnelLifecycleProducer, PositionManagementProducer};
use rusty_meshed_sdk::DataProductProducerBase;
use std::collections::BTreeMap;

/// Builds the fixed, non-random 14-event demo scenario (DOM-046): 3
/// people activated, 3 positions authorized under `UNIT-ALPHA`, all 3
/// assigned and filled, one promotion, and one retroactive correction
/// on P1's assignment.
fn build_demo_scenario() -> (String, Vec<ScenarioEvent>) {
    let builder = ScenarioBuilder::new();

    let builder = builder
        .add_status_change("P1", "ACTIVE", "NONE", 1)
        .add_status_change("P2", "ACTIVE", "NONE", 1)
        .add_status_change("P3", "ACTIVE", "NONE", 1)
        .add_position_authorization("POS1", "UNIT-ALPHA", "E5", "Team Leader", 1)
        .add_position_authorization("POS2", "UNIT-ALPHA", "E4", "Rifleman", 1)
        .add_position_authorization("POS3", "UNIT-ALPHA", "E6", "Squad Leader", 1);

    let builder = builder
        .add_assignment("P1", "POS1", "UNIT-ALPHA", "Team Leader", "E5", 1)
        .expect("P1 was just activated above")
        .add_assignment("P2", "POS2", "UNIT-ALPHA", "Rifleman", "E4", 1)
        .expect("P2 was just activated above")
        .add_assignment("P3", "POS3", "UNIT-ALPHA", "Squad Leader", "E6", 1)
        .expect("P3 was just activated above");

    let builder = builder
        .add_position_fill("POS1", "P1", "UNIT-ALPHA", 1)
        .expect("POS1 was just authorized above")
        .add_position_fill("POS2", "P2", "UNIT-ALPHA", 1)
        .expect("POS2 was just authorized above")
        .add_position_fill("POS3", "P3", "UNIT-ALPHA", 1)
        .expect("POS3 was just authorized above");

    let builder = builder
        .add_promotion("P3", "E6", "E7", 5)
        .add_retroactive_correction("P1", "POS1", "UNIT-ALPHA", "Team Leader", "E5", 30, 1);

    let correlation_id = builder.correlation_id().to_string();
    (correlation_id, builder.build())
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

#[rusty_tokio::main]
async fn main() {
    let kafka_bootstrap = env_or("KAFKA_BOOTSTRAP_SERVERS", "localhost:9092");
    let schema_registry_url = env_or("SCHEMA_REGISTRY_URL", "http://localhost:8081");
    let registry_api_url = env_or("REGISTRY_API_URL", "http://localhost:8000");

    println!("=== Meshed Phase-4 Cross-Domain Scenario Demo ===");
    println!("Kafka:           {kafka_bootstrap}");
    println!("Schema Registry: {schema_registry_url}");
    println!("Registry API:    {registry_api_url}");

    // -- Step 1: Create topics -----------------------------------------
    println!("--- Step 1: Creating Phase-4 topics ---");
    let mut admin_client =
        match rusty_kafka::KafkaClient::connect(&kafka_bootstrap, Some("run_scenario".to_string()))
            .await
        {
            Ok(client) => client,
            Err(err) => {
                eprintln!(
                    "Cannot reach Kafka at {kafka_bootstrap}: {err}\n\
                     Ensure the infrastructure stack is running (podman-compose up).\n\
                     Exiting."
                );
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
            eprintln!(
                "Cannot reach Kafka at {kafka_bootstrap}: {err}\n\
                 Ensure the infrastructure stack is running (podman-compose up).\n\
                 Exiting."
            );
            std::process::exit(1);
        }
    }

    // -- Step 2: Build demo scenario -------------------------------------
    println!("--- Step 2: Building demo scenario ---");
    let (correlation_id, events) = build_demo_scenario();
    println!("Scenario correlation_id: {correlation_id}");
    println!("Total events generated: {}", events.len());
    for event in &events {
        let id = &event.base().event_id[..event.base().event_id.len().min(8)];
        println!(
            "  [{}] {} — event_id={id}",
            event.event_name(),
            event.person_or_position_id()
        );
    }

    // -- Step 3: Instantiate producers and publish events ------------------
    println!("--- Step 3: Publishing events ---");
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

    let mut published: BTreeMap<&'static str, u64> = BTreeMap::new();
    for event in &events {
        let Some(topic) = event_topic(event.event_name()) else {
            eprintln!(
                "No topic mapping for event type {} — skipping.",
                event.event_name()
            );
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
        *published.entry(topic).or_insert(0) += 1;
        println!("Published {} -> {topic}", event.event_name());
    }
    position_producer.flush(10.0);

    // -- Step 4: Print summary --------------------------------------------
    println!("--- Step 4: Summary ---");
    println!("Correlation ID: {correlation_id}");
    println!("Events published per topic:");
    for (topic, count) in &published {
        println!("  {topic:<55}  {count}");
    }
    println!("Total published: {}", published.values().sum::<u64>());
    println!("=== Demo complete ===");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_demo_scenario_produces_exactly_fourteen_events_in_source_order() {
        let (_correlation_id, events) = build_demo_scenario();
        let names: Vec<&str> = events.iter().map(ScenarioEvent::event_name).collect();
        assert_eq!(
            names,
            vec![
                "StatusChanged",
                "StatusChanged",
                "StatusChanged",
                "PositionAuthorizationChanged",
                "PositionAuthorizationChanged",
                "PositionAuthorizationChanged",
                "PersonnelAssigned",
                "PersonnelAssigned",
                "PersonnelAssigned",
                "PositionFilled",
                "PositionFilled",
                "PositionFilled",
                "PersonnelPromoted",
                "PersonnelAssigned",
            ]
        );
    }

    #[test]
    fn build_demo_scenario_events_all_share_one_correlation_id() {
        let (correlation_id, events) = build_demo_scenario();
        for event in &events {
            assert_eq!(event.base().correlation_id, correlation_id);
        }
    }

    #[test]
    fn build_demo_scenario_final_event_is_the_retroactive_correction() {
        let (_correlation_id, events) = build_demo_scenario();
        match events.last().unwrap() {
            ScenarioEvent::PersonnelAssigned(a) => {
                assert_eq!(a.person_id, "P1");
                assert!(a.effective_date < a.transaction_date);
            }
            other => panic!("expected PersonnelAssigned, got {other:?}"),
        }
    }

    #[test]
    fn build_demo_scenario_position_fills_link_back_to_their_assignments() {
        let (_correlation_id, events) = build_demo_scenario();
        let assigned_event_id = |person_id: &str| {
            events
                .iter()
                .find_map(|e| match e {
                    ScenarioEvent::PersonnelAssigned(a) if a.person_id == person_id => {
                        Some(a.base.event_id.clone())
                    }
                    _ => None,
                })
                .unwrap()
        };
        for (position, person) in [("POS1", "P1"), ("POS2", "P2"), ("POS3", "P3")] {
            let filled = events
                .iter()
                .find_map(|e| match e {
                    ScenarioEvent::PositionFilled(f) if f.position_id == position => Some(f),
                    _ => None,
                })
                .unwrap();
            assert!(filled
                .base
                .source_event_ids
                .contains(&assigned_event_id(person)));
        }
    }
}

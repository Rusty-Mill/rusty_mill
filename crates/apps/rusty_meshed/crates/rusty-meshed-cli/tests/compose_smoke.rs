//! Smoke tests for the local-dev compose stack (CLI-051..053) -- the
//! Rust port of the source's `tests/test_compose.py`. These require a
//! running `docker compose up -d`/`podman-compose up -d` stack (see
//! `../../compose.yaml`) and are skipped by default, matching the
//! source's own `pytest.mark.skipif(not os.environ.get("MESHED_COMPOSE_UP"))`.
//!
//! To run them against a live stack:
//! `MESHED_COMPOSE_UP=1 cargo test -p rusty-meshed-cli --test compose_smoke`
//!
//! Rust has no first-class conditional-skip attribute the way pytest's
//! `skipif` does, so each test checks the same env var itself and
//! returns early (printing why) rather than failing -- this sandbox
//! has no docker/podman available at all, so every test here always
//! takes that early-return path, but the checks themselves are exactly
//! what a real compose stack would be tested against.

fn compose_up() -> bool {
    std::env::var("MESHED_COMPOSE_UP")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

/// `AdminClient.list_topics(timeout=10)` must not raise (CLI-051) --
/// here, `ApiVersions` against the broker must succeed.
#[rusty_tokio::test]
async fn kafka_reachable() {
    if !compose_up() {
        eprintln!("skipped: set MESHED_COMPOSE_UP=1 to run against a live compose stack");
        return;
    }
    let mut client = rusty_kafka::KafkaClient::connect(
        "localhost:9092",
        Some("rusty_meshed_compose_smoke".to_string()),
    )
    .await
    .expect("Kafka broker at localhost:9092 must be reachable");
    client
        .api_versions()
        .await
        .expect("ApiVersions request must succeed");
}

/// Schema Registry `/subjects` endpoint returns HTTP 200 (CLI-052).
#[rusty_tokio::test]
async fn schema_registry_health() {
    if !compose_up() {
        eprintln!("skipped: set MESHED_COMPOSE_UP=1 to run against a live compose stack");
        return;
    }
    let response = rusty_request::Client::new()
        .get("http://localhost:8081/subjects")
        .unwrap()
        .send()
        .await
        .expect("Schema Registry must be reachable");
    assert_eq!(response.status().as_u16(), 200);
}

/// Kafka UI root endpoint returns HTTP 200 (CLI-053).
#[rusty_tokio::test]
async fn kafka_ui_reachable() {
    if !compose_up() {
        eprintln!("skipped: set MESHED_COMPOSE_UP=1 to run against a live compose stack");
        return;
    }
    let response = rusty_request::Client::new()
        .get("http://localhost:8080")
        .unwrap()
        .send()
        .await
        .expect("Kafka UI must be reachable");
    assert_eq!(response.status().as_u16(), 200);
}

//! Unit tests for the service inventory.

use crate::Config;

fn config(source: &str) -> Config {
    Config::from_yaml(source).expect("should parse")
}

const INVENTORY: &str = r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - backends:
              - service:
                  name: default/echo.default.svc.cluster.local
                  port: 80
services:
  - name: echo
    namespace: default
    hostname: echo.default.svc.cluster.local
    ports:
      80: 8080
workloads:
  - name: echo-1
    namespace: default
    workloadIps: ["10.0.0.1"]
    services:
      "default/echo.default.svc.cluster.local":
        80: 8080
"#;

#[test]
fn an_inventory_parses_beside_the_binds() {
    let config = config(INVENTORY);
    assert_eq!(config.services.len(), 1);
    assert_eq!(config.workloads.len(), 1);
    assert_eq!(
        config.services[0].key(),
        "default/echo.default.svc.cluster.local"
    );
    assert_eq!(config.services[0].target_port(80), Some(8080));
    assert_eq!(config.workloads[0].address(), Some("10.0.0.1"));
}

#[test]
fn a_port_map_reads_integer_and_quoted_keys_alike() {
    // YAML writes `80: 8080` and JSON cannot -- its object keys are strings --
    // so a file converted from one to the other would stop parsing.
    let quoted = config(
        r#"
binds:
  - port: 3000
    listeners: []
services:
  - name: echo
    namespace: default
    ports:
      "80": 8080
"#,
    );
    assert_eq!(quoted.services[0].target_port(80), Some(8080));
}

#[test]
fn a_port_that_is_not_a_port_is_refused() {
    let err = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners: []
services:
  - name: echo
    ports:
      not-a-port: 8080
"#,
    )
    .expect_err("should not parse");
    assert!(err.to_string().contains("not-a-port"), "got: {err}");
}

#[test]
fn a_service_with_no_port_map_answers_on_the_port_it_was_asked_for() {
    // Upstream's own local examples leave the map off when the service and
    // target ports are the same, so an empty map maps nothing rather than
    // answering nothing.
    let bare = config(
        r#"
binds:
  - port: 3000
    listeners: []
services:
  - name: echo
    namespace: default
"#,
    );
    assert_eq!(bare.services[0].target_port(9999), Some(9999));
}

#[test]
fn a_mapped_service_answers_only_on_the_ports_it_lists() {
    let config = config(INVENTORY);
    assert_eq!(config.services[0].target_port(80), Some(8080));
    assert_eq!(
        config.services[0].target_port(443),
        None,
        "not answering on a port is different from answering unmapped"
    );
}

#[test]
fn a_hostname_falls_back_to_the_name() {
    // A hand-written entry usually has one or the other.
    let named = config(
        r#"
binds:
  - port: 3000
    listeners: []
services:
  - name: echo
    namespace: default
"#,
    );
    assert_eq!(named.services[0].key(), "default/echo");
}

#[test]
fn a_workload_with_no_address_can_still_be_a_name() {
    let external = config(
        r#"
binds:
  - port: 3000
    listeners: []
workloads:
  - name: external
    hostname: api.example.com
"#,
    );
    assert_eq!(external.workloads[0].address(), Some("api.example.com"));

    let nowhere = config(
        r#"
binds:
  - port: 3000
    listeners: []
workloads:
  - name: nowhere
"#,
    );
    assert_eq!(nowhere.workloads[0].address(), None);
}

#[test]
fn health_defaults_to_healthy() {
    // An entry written down without a status is one somebody expects used.
    assert_eq!(
        config(INVENTORY).workloads[0].status,
        crate::Health::Healthy
    );

    let marked = config(
        r#"
binds:
  - port: 3000
    listeners: []
workloads:
  - name: echo-1
    status: unhealthy
"#,
    );
    assert_eq!(marked.workloads[0].status, crate::Health::Unhealthy);
}

#[test]
fn mesh_only_fields_parse_and_are_reported() {
    let config = config(
        r#"
binds:
  - port: 3000
    listeners: []
services:
  - name: echo
    namespace: default
    vips: ["/10.96.0.1"]
    waypoint:
      hostname: waypoint.default
workloads:
  - name: echo-1
    workloadIps: ["10.0.0.1"]
    locality:
      region: us-east-1
"#,
    );

    let findings = config.lint();
    for expected in [
        "services[0].vips",
        "services[0].waypoint",
        "workloads[0].locality",
    ] {
        assert!(
            findings.iter().any(|f| f.contains(expected)),
            "{expected} should be reported: {findings:?}"
        );
    }
    assert!(
        findings.iter().any(|f| f.contains("mesh")),
        "the finding should say why: {findings:?}"
    );
}

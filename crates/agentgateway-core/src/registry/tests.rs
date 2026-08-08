//! Unit tests for service resolution.

use agentgateway_config::{Config, ServicePorts};

use super::*;

/// Two services, one with two instances and one with one.
const TWO: &str = r#"
binds:
  - port: 3000
    listeners: []
services:
  - name: echo
    namespace: default
    hostname: echo.default.svc.cluster.local
    ports:
      80: 8080
  - name: api
    namespace: default
    hostname: api.default.svc.cluster.local
    ports:
      443: 8443
workloads:
  - name: echo-1
    namespace: default
    workloadIps: ["10.0.0.1"]
    services:
      "default/echo.default.svc.cluster.local": {}
  - name: echo-2
    namespace: default
    workloadIps: ["10.0.0.2"]
    services:
      "default/echo.default.svc.cluster.local": {}
  - name: api-1
    namespace: default
    workloadIps: ["10.0.1.1"]
    services:
      "default/api.default.svc.cluster.local": {}
"#;

fn registry(source: &str) -> Registry {
    let config = Config::from_yaml(source).expect("should parse");
    Registry::new(&config.services, &config.workloads, &config.backends)
}

fn reference(name: &str, port: u16) -> ServiceRef {
    ServiceRef {
        name: name.to_string(),
        port,
    }
}

#[test]
fn a_service_resolves_to_every_instance_that_named_it() {
    // A workload claims membership; nothing in the service names its
    // endpoints. That is what lets instances come and go.
    let registry = registry(TWO);
    let mut addresses = registry
        .resolve(
            &reference("default/echo.default.svc.cluster.local", 80),
            "route[0]",
        )
        .expect("should resolve");
    addresses.sort();
    assert_eq!(addresses, vec!["10.0.0.1:8080", "10.0.0.2:8080"]);
}

#[test]
fn the_service_port_is_mapped_to_the_target_port() {
    let registry = registry(TWO);
    let addresses = registry
        .resolve(
            &reference("default/api.default.svc.cluster.local", 443),
            "route[0]",
        )
        .expect("should resolve");
    assert_eq!(addresses, vec!["10.0.1.1:8443"]);
}

#[test]
fn a_workloads_own_port_wins_over_the_services() {
    // How one instance in a set differs from the rest.
    let registry = registry(
        r#"
binds:
  - port: 3000
    listeners: []
services:
  - name: echo
    namespace: default
    ports:
      80: 8080
workloads:
  - name: usual
    workloadIps: ["10.0.0.1"]
    services:
      "default/echo": {}
  - name: odd-one-out
    workloadIps: ["10.0.0.2"]
    services:
      "default/echo":
        80: 9999
"#,
    );

    let mut addresses = registry
        .resolve(&reference("default/echo", 80), "route[0]")
        .expect("should resolve");
    addresses.sort();
    assert_eq!(addresses, vec!["10.0.0.1:8080", "10.0.0.2:9999"]);
}

#[test]
fn a_service_can_be_named_three_ways() {
    let registry = registry(TWO);
    for name in [
        "default/echo.default.svc.cluster.local",
        "echo.default.svc.cluster.local",
        "echo",
    ] {
        assert!(
            registry.resolve(&reference(name, 80), "route[0]").is_ok(),
            "`{name}` should resolve"
        );
    }
}

#[test]
fn a_short_name_two_namespaces_share_is_ambiguous_rather_than_sorted() {
    let registry = registry(
        r#"
binds:
  - port: 3000
    listeners: []
services:
  - name: api
    namespace: alpha
    hostname: api.alpha.svc
  - name: api
    namespace: beta
    hostname: api.beta.svc
workloads:
  - name: a
    workloadIps: ["10.0.0.1"]
    services:
      "alpha/api.alpha.svc": {}
  - name: b
    workloadIps: ["10.0.0.2"]
    services:
      "beta/api.beta.svc": {}
"#,
    );

    let err = registry
        .resolve(&reference("api", 80), "route[0]")
        .expect_err("two services answer to `api`");
    assert!(err.to_string().contains("matches 2"), "got: {err}");
    assert!(err.to_string().contains("namespace/hostname"), "got: {err}");

    // Named fully, it resolves.
    assert!(
        registry
            .resolve(&reference("alpha/api.alpha.svc", 80), "r")
            .is_ok()
    );
}

#[test]
fn an_unknown_service_names_what_the_inventory_does_hold() {
    let registry = registry(TWO);
    let err = registry
        .resolve(&reference("nope", 80), "route[0]")
        .expect_err("should not resolve");
    let message = err.to_string();
    assert!(message.contains("route[0]"), "{message}");
    assert!(message.contains("`nope`"), "{message}");
    assert!(
        message.contains("default/echo.default.svc.cluster.local"),
        "the fix is easier to see with the list: {message}"
    );
}

#[test]
fn a_port_the_service_does_not_answer_on_says_which_it_does() {
    let registry = registry(TWO);
    let err = registry
        .resolve(&reference("echo", 9999), "route[0]")
        .expect_err("should not resolve");
    let message = err.to_string();
    assert!(message.contains("9999"), "{message}");
    assert!(message.contains("answers on 80"), "{message}");
}

#[test]
fn a_service_nothing_backs_is_refused_at_startup() {
    // A route pointed at one would answer every call with an error, and in a
    // file-driven inventory nothing is going to come along and fill it in.
    let registry = registry(
        r#"
binds:
  - port: 3000
    listeners: []
services:
  - name: lonely
    namespace: default
"#,
    );
    let err = registry
        .resolve(&reference("lonely", 80), "route[0]")
        .expect_err("should not resolve");
    assert!(
        err.to_string().contains("no healthy instance"),
        "got: {err}"
    );
}

#[test]
fn an_unhealthy_instance_is_left_out_of_the_set() {
    let registry = registry(
        r#"
binds:
  - port: 3000
    listeners: []
services:
  - name: echo
    namespace: default
workloads:
  - name: good
    workloadIps: ["10.0.0.1"]
    services:
      "default/echo": {}
  - name: bad
    workloadIps: ["10.0.0.2"]
    status: unhealthy
    services:
      "default/echo": {}
"#,
    );
    assert_eq!(
        registry
            .resolve(&reference("echo", 80), "route[0]")
            .expect("should resolve"),
        vec!["10.0.0.1:80"]
    );
}

#[test]
fn a_service_whose_instances_are_all_unhealthy_is_refused() {
    let registry = registry(
        r#"
binds:
  - port: 3000
    listeners: []
services:
  - name: echo
    namespace: default
workloads:
  - name: bad
    workloadIps: ["10.0.0.1"]
    status: unhealthy
    services:
      "default/echo": {}
"#,
    );
    assert!(registry.resolve(&reference("echo", 80), "r").is_err());
}

fn backend(target: BackendTarget, weight: u32) -> Backend {
    Backend { target, weight }
}

#[test]
fn one_service_splits_its_traffic_evenly() {
    let registry = registry(TWO);
    let resolved = resolve_backends(
        &[backend(BackendTarget::Service(reference("echo", 80)), 1)],
        &registry,
        "route[0]",
    )
    .expect("should resolve");

    assert_eq!(resolved.len(), 2);
    assert!(
        resolved
            .iter()
            .all(|endpoint| endpoint.weight == resolved[0].weight),
        "{resolved:?}"
    );
}

#[test]
fn a_service_beside_a_host_still_gets_exactly_half() {
    // The reason weights are scaled rather than repeated: three instances
    // would otherwise take three quarters of the route.
    let registry = registry(
        r#"
binds:
  - port: 3000
    listeners: []
services:
  - name: echo
    namespace: default
workloads:
  - name: a
    workloadIps: ["10.0.0.1"]
    services: {"default/echo": {}}
  - name: b
    workloadIps: ["10.0.0.2"]
    services: {"default/echo": {}}
  - name: c
    workloadIps: ["10.0.0.3"]
    services: {"default/echo": {}}
"#,
    );

    let resolved = resolve_backends(
        &[
            backend(BackendTarget::Service(reference("echo", 80)), 1),
            backend(BackendTarget::Host("literal:80".into()), 1),
        ],
        &registry,
        "route[0]",
    )
    .expect("should resolve");

    let service_total: u32 = resolved
        .iter()
        .filter(|endpoint| endpoint.authority.starts_with("10.0.0."))
        .map(|endpoint| endpoint.weight)
        .sum();
    let host_total: u32 = resolved
        .iter()
        .filter(|endpoint| endpoint.authority == "literal:80")
        .map(|endpoint| endpoint.weight)
        .sum();
    assert_eq!(service_total, host_total, "{resolved:?}");
}

#[test]
fn two_services_of_different_sizes_keep_their_ratio() {
    let registry = registry(TWO);
    let resolved = resolve_backends(
        &[
            // Two instances, weight 1.
            backend(BackendTarget::Service(reference("echo", 80)), 1),
            // One instance, weight 3.
            backend(BackendTarget::Service(reference("api", 443)), 3),
        ],
        &registry,
        "route[0]",
    )
    .expect("should resolve");

    let echo: u32 = resolved
        .iter()
        .filter(|endpoint| endpoint.authority.starts_with("10.0.0."))
        .map(|endpoint| endpoint.weight)
        .sum();
    let api: u32 = resolved
        .iter()
        .filter(|endpoint| endpoint.authority.starts_with("10.0.1."))
        .map(|endpoint| endpoint.weight)
        .sum();
    assert_eq!(
        api,
        echo * 3,
        "1:3 whatever the instance counts: {resolved:?}"
    );
}

#[test]
fn a_drained_backend_stays_drained() {
    // Weight 0 is how a backend is taken out of rotation without deleting its
    // configuration, and scaling must not lift it back in.
    let registry = registry(TWO);
    let resolved = resolve_backends(
        &[
            backend(BackendTarget::Service(reference("echo", 80)), 0),
            backend(BackendTarget::Host("literal:80".into()), 1),
        ],
        &registry,
        "route[0]",
    )
    .expect("should resolve");

    for endpoint in &resolved {
        if endpoint.authority.starts_with("10.0.0.") {
            assert_eq!(endpoint.weight, 0, "{resolved:?}");
        }
    }
}

#[test]
fn a_route_of_only_hosts_is_unchanged_by_resolution() {
    let registry = Registry::default();
    let resolved = resolve_backends(
        &[
            backend(BackendTarget::Host("a:80".into()), 1),
            backend(BackendTarget::Host("b:80".into()), 9),
        ],
        &registry,
        "route[0]",
    )
    .expect("should resolve");

    assert_eq!(
        resolved,
        vec![
            Endpoint {
                authority: "a:80".into(),
                weight: 1
            },
            Endpoint {
                authority: "b:80".into(),
                weight: 9
            },
        ]
    );
}

#[test]
fn a_workload_with_no_service_map_backs_nothing() {
    let registry = registry(
        r#"
binds:
  - port: 3000
    listeners: []
services:
  - name: echo
    namespace: default
workloads:
  - name: unrelated
    workloadIps: ["10.0.0.1"]
"#,
    );
    assert!(registry.resolve(&reference("echo", 80), "r").is_err());
}

#[test]
fn a_named_backend_resolves_to_its_address() {
    let registry = registry(
        r#"
binds:
  - port: 3000
    listeners: []
backends:
  - name: guard
    host: guard.internal:9000
"#,
    );
    assert_eq!(
        registry
            .backend("guard", "route[0]")
            .expect("should resolve"),
        "guard.internal:9000"
    );
}

#[test]
fn an_unknown_backend_names_what_the_list_does_hold() {
    let registry = registry(
        r#"
binds:
  - port: 3000
    listeners: []
backends:
  - name: guard
    host: guard.internal:9000
"#,
    );
    let err = registry
        .backend("nope", "route[0]")
        .expect_err("should not resolve");
    assert!(err.to_string().contains("`nope`"), "{err}");
    assert!(err.to_string().contains("guard"), "{err}");
}

#[test]
fn a_backend_with_no_address_cannot_be_dialled() {
    // An `mcp` or `ai` backend is not an address, and something referring to
    // one needs to be told so rather than dialling nothing.
    let registry = registry(
        r#"
binds:
  - port: 3000
    listeners: []
backends:
  - name: federated
    mcp:
      targets: []
"#,
    );
    let err = registry
        .backend("federated", "route[0]")
        .expect_err("should not resolve");
    assert!(err.to_string().contains("no `host`"), "{err}");
}

#[test]
fn service_ports_written_as_a_map_are_read_back() {
    // Guards the newtype: a workload's per-service map is the one place the
    // port map is nested two deep.
    let ports = ServicePorts([(80u16, 8080u16)].into_iter().collect());
    assert_eq!(ports.0.get(&80), Some(&8080));
}

//! Unit tests for passthrough routing.
//!
//! The forwarding half is exercised end to end elsewhere; what is worth
//! pinning here is which route a name picks, because that is the only routing
//! decision a listener nobody reads can make.

use agentgateway_config::{Backend, BackendTarget};

use super::*;

fn route(hostnames: &[&str], host: &str) -> TcpRoute {
    TcpRoute {
        name: None,
        hostnames: hostnames.iter().map(|h| (*h).to_string()).collect(),
        backends: vec![Backend {
            target: BackendTarget::Host(host.to_string()),
            weight: 1,
        }],
        rest: Default::default(),
    }
}

fn passthrough(routes: Vec<TcpRoute>) -> Passthrough {
    Passthrough::new(&routes, &Registry::default(), "binds[0]").expect("should compile")
}

#[test]
fn a_name_picks_the_route_that_claims_it() {
    let forwarder = passthrough(vec![
        route(&["alpha.test"], "10.0.0.1:443"),
        route(&["beta.test"], "10.0.0.2:443"),
    ]);

    assert_eq!(
        forwarder.destination(Some("alpha.test")).as_deref(),
        Some("10.0.0.1:443")
    );
    assert_eq!(
        forwarder.destination(Some("beta.test")).as_deref(),
        Some("10.0.0.2:443")
    );
}

#[test]
fn an_exact_name_wins_over_a_wildcard() {
    // Written in the order that would give the wrong answer without sorting.
    let forwarder = passthrough(vec![
        route(&["*.example.test"], "10.0.0.9:443"),
        route(&["api.example.test"], "10.0.0.1:443"),
    ]);

    assert_eq!(
        forwarder.destination(Some("api.example.test")).as_deref(),
        Some("10.0.0.1:443")
    );
    assert_eq!(
        forwarder.destination(Some("other.example.test")).as_deref(),
        Some("10.0.0.9:443")
    );
}

#[test]
fn a_route_with_no_hostnames_takes_what_is_left() {
    let forwarder = passthrough(vec![
        route(&["alpha.test"], "10.0.0.1:443"),
        route(&[], "10.0.0.9:443"),
    ]);

    assert_eq!(
        forwarder.destination(Some("alpha.test")).as_deref(),
        Some("10.0.0.1:443")
    );
    assert_eq!(
        forwarder.destination(Some("stranger.test")).as_deref(),
        Some("10.0.0.9:443"),
        "the catch-all takes a name nothing else claims"
    );
}

#[test]
fn a_connection_with_no_name_only_matches_the_catch_all() {
    // A client dialling by IP, or something that is not TLS at all.
    let named = passthrough(vec![route(&["alpha.test"], "10.0.0.1:443")]);
    assert_eq!(
        named.destination(None),
        None,
        "a named route has no name to compare against"
    );

    let with_default = passthrough(vec![
        route(&["alpha.test"], "10.0.0.1:443"),
        route(&[], "10.0.0.9:443"),
    ]);
    assert_eq!(
        with_default.destination(None).as_deref(),
        Some("10.0.0.9:443")
    );
}

#[test]
fn a_name_nothing_claims_is_closed_rather_than_guessed() {
    // Sending it to whichever route sorted first would forward a connection
    // to somewhere the operator never pointed that name.
    let forwarder = passthrough(vec![
        route(&["alpha.test"], "10.0.0.1:443"),
        route(&["beta.test"], "10.0.0.2:443"),
    ]);
    assert_eq!(forwarder.destination(Some("stranger.test")), None);
}

#[test]
fn several_backends_share_the_connections() {
    let forwarder = passthrough(vec![TcpRoute {
        name: None,
        hostnames: vec!["alpha.test".into()],
        backends: vec![
            Backend {
                target: BackendTarget::Host("10.0.0.1:443".into()),
                weight: 1,
            },
            Backend {
                target: BackendTarget::Host("10.0.0.2:443".into()),
                weight: 1,
            },
        ],
        rest: Default::default(),
    }]);

    let picked: Vec<String> = (0..4)
        .filter_map(|_| forwarder.destination(Some("alpha.test")))
        .collect();
    assert_eq!(
        picked,
        vec![
            "10.0.0.1:443",
            "10.0.0.2:443",
            "10.0.0.1:443",
            "10.0.0.2:443"
        ],
        "round-robin, the same ring every other backend uses"
    );
}

#[test]
fn a_route_with_no_backends_does_not_compile() {
    // A route that claims a name and forwards it nowhere would close every
    // connection asking for that name, which is worse than not claiming it.
    let empty = TcpRoute {
        name: None,
        hostnames: vec!["alpha.test".into()],
        backends: Vec::new(),
        rest: Default::default(),
    };
    assert!(Passthrough::new(&[empty], &Registry::default(), "binds[0]").is_err());
}

#[test]
fn nothing_configured_forwards_nothing() {
    assert!(passthrough(Vec::new()).is_empty());
}

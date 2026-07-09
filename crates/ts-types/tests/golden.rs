//! Golden tests: decode JSON captured from a live tailscaled 1.86.2 on a
//! real Headscale 0.26 tailnet (see `tests/fixtures/README.md` for how the
//! fixtures were produced) and pin every field the workspace reads.
//! Round-trips re-encode and re-decode to prove our serialization is
//! self-consistent and loses none of the modeled fields.

use ts_types::{MaskedPrefs, NodePublic, PingResult, Prefs, Status, UserID};

const STATUS_JSON: &str = include_str!("fixtures/status.json");
const PING_JSON: &str = include_str!("fixtures/ping.json");
const PREFS_JSON: &str = include_str!("fixtures/prefs.json");

#[test]
fn status_golden() {
    let st: Status = serde_json::from_str(STATUS_JSON).expect("decode status fixture");

    assert_eq!(st.version, "1.86.2-tc47caa10d-gf5d087d04");
    assert!(!st.tun, "userspace-networking node");
    assert_eq!(st.backend_state, "Running");
    assert!(st.have_node_key);
    assert_eq!(st.auth_url, "");
    assert_eq!(st.tailscale_ips.len(), 2);
    assert_eq!(
        st.tailscale_ips[0],
        "100.64.0.1".parse::<std::net::IpAddr>().unwrap()
    );
    assert_eq!(st.magic_dns_suffix, "tailnet.test");
    assert_eq!(
        st.cert_domains,
        Vec::<String>::new(),
        "null decodes as empty"
    );

    let tailnet = st.current_tailnet.as_ref().expect("CurrentTailnet");
    assert!(tailnet.magic_dns_enabled);
    assert_eq!(tailnet.magic_dns_suffix, "tailnet.test");

    // Self
    let self_ = st.self_.as_ref().expect("Self");
    assert_eq!(self_.host_name, "node1");
    assert_eq!(self_.dns_name, "node1.tailnet.test.");
    assert_eq!(self_.name(), "node1");
    assert_eq!(self_.os, "linux");
    assert_eq!(self_.user_id, UserID(1));
    assert_eq!(self_.id.0, "1");
    assert_eq!(
        self_.public_key.unwrap().to_string(),
        "nodekey:43b662bffd68e54a8f31f88d2ed52f445df297567de6ae08a4692f53cee68c13"
    );
    assert_eq!(self_.primary_ip().unwrap().to_string(), "100.64.0.1");
    assert_eq!(self_.allowed_ips.len(), 2);
    assert_eq!(self_.allowed_ips[0].to_string(), "100.64.0.1/32");
    assert_eq!(self_.addrs.len(), 3, "self has advertised endpoints");
    assert_eq!(self_.relay, "headscale");
    assert!(self_.online);
    assert!(self_.created.0.starts_with("2026-07-09T"));
    assert!(self_.last_handshake.is_zero());
    assert!(self_.cap_map.contains_key("https://tailscale.com/cap/ssh"));

    // Peers (keyed by node public key)
    assert_eq!(st.peer.len(), 1);
    let key: NodePublic =
        "nodekey:80653c0b63c66894720f079ebb156a374b3de84bf84952fed2ea478d706f1571"
            .parse()
            .unwrap();
    let peer = st.peer.get(&key).expect("peer under its node key");
    assert_eq!(peer.host_name, "node2");
    assert_eq!(peer.name(), "node2");
    assert!(peer.online);
    assert!(peer.active);
    assert_eq!(peer.cur_addr, "127.0.0.1:41642", "direct path endpoint");
    assert_eq!(
        peer.addrs,
        Vec::<String>::new(),
        "null Addrs decodes as empty"
    );
    assert_eq!(st.sorted_peers()[0].host_name, "node2");

    // Users (map key is a *string* user ID)
    let user = st.user.get(&UserID(1)).expect("user 1");
    assert_eq!(user.login_name, "interop");

    // Health strings surface as-is.
    assert_eq!(st.health.len(), 3);

    round_trip(&st);
}

#[test]
fn ping_golden() {
    let pr: PingResult = serde_json::from_str(PING_JSON).expect("decode ping fixture");
    assert_eq!(pr.ip, "100.64.0.2");
    assert_eq!(pr.node_ip, "100.64.0.2");
    assert_eq!(pr.node_name, "node2");
    assert_eq!(pr.err, "");
    assert!(pr.latency_seconds > 0.0);
    assert_eq!(pr.endpoint, "127.0.0.1:41642");
    assert_eq!(pr.derp_region_id, 0);
    round_trip(&pr);
}

#[test]
fn prefs_golden() {
    let p: Prefs = serde_json::from_str(PREFS_JSON).expect("decode prefs fixture");
    assert_eq!(p.control_url, "http://127.0.0.1:8080");
    assert!(p.want_running);
    assert!(!p.logged_out);
    assert!(p.corp_dns);
    assert_eq!(p.hostname, "node1");
    assert_eq!(
        p.advertise_tags,
        Vec::<String>::new(),
        "null decodes as empty"
    );
    assert_eq!(p.netfilter_mode, 2);
    round_trip(&p);
}

#[test]
fn masked_prefs_matches_go_cli_shape() {
    // What `tailscale down` PATCHes: only masked fields appear.
    let mp = MaskedPrefs {
        want_running: Some(false),
    };
    let v: serde_json::Value = serde_json::to_value(&mp).unwrap();
    assert_eq!(
        v,
        serde_json::json!({"WantRunning": false, "WantRunningSet": true})
    );
}

/// Arbitrary/hostile input must error, never panic.
#[test]
fn decoding_is_panic_free_on_garbage() {
    for garbage in [
        "",
        "null",
        "[]",
        "{\"Peer\": {\"not-a-key\": {}}}",
        "{\"TailscaleIPs\": [\"bogus\"]}",
        "{\"Self\": {\"AllowedIPs\": [\"10.0.0.0/99\"]}}",
        "{\"User\": {\"NaN\": {}}}",
    ] {
        let _ = serde_json::from_str::<Status>(garbage);
        let _ = serde_json::from_str::<PingResult>(garbage);
        let _ = serde_json::from_str::<Prefs>(garbage);
    }
    // Deep nesting must not blow the stack (serde_json has a recursion limit).
    let deep = format!(
        "{}{}",
        "{\"Peer\":".repeat(200),
        "1".to_owned() + &"}".repeat(200)
    );
    let _ = serde_json::from_str::<Status>(&deep);
}

/// Missing and null-heavy documents decode to defaults.
#[test]
fn minimal_documents_decode() {
    let st: Status = serde_json::from_str("{}").unwrap();
    assert_eq!(st.backend_state, "");
    assert!(st.self_.is_none());
    assert!(st.peer.is_empty());

    let st: Status =
        serde_json::from_str(r#"{"Peer": null, "User": null, "Health": null}"#).unwrap();
    assert!(st.peer.is_empty() && st.user.is_empty() && st.health.is_empty());
}

fn round_trip<T>(value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let encoded = serde_json::to_string(value).expect("re-encode");
    let decoded: T = serde_json::from_str(&encoded).expect("re-decode");
    assert_eq!(&decoded, value, "round-trip must be lossless");
}

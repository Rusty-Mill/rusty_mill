//! Human-readable rendering of LocalAPI responses, following the layout of
//! `tailscale status` (a simplified subset of the Go CLI's link-state
//! wording; see DESIGN.md "Deviations").

use ts_types::{PeerStatus, PingResult, Status};

/// Renders like `tailscale status`: one line per node —
/// IP, name, owner, OS, link state.
pub fn status(st: &Status) -> String {
    let mut out = String::new();

    if st.backend_state != "Running" {
        out.push_str(&format!("# backend state: {}\n", st.backend_state));
        if !st.auth_url.is_empty() {
            out.push_str(&format!("# log in at: {}\n", st.auth_url));
        }
        if st.self_.is_none() {
            return out;
        }
    }

    if let Some(self_) = &st.self_ {
        out.push_str(&line(st, self_, "-"));
    }
    for peer in st.sorted_peers() {
        let state = peer_state(peer);
        out.push_str(&line(st, peer, &state));
    }

    if !st.health.is_empty() {
        out.push_str("\n# Health check:\n");
        for h in &st.health {
            out.push_str(&format!("#     - {h}\n"));
        }
    }
    out
}

fn line(st: &Status, ps: &PeerStatus, state: &str) -> String {
    let ip = ps
        .primary_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "-".into());
    let owner = st
        .user
        .get(&ps.user_id)
        .map(|u| {
            if u.login_name.contains('@') {
                u.login_name.clone()
            } else {
                format!("{}@", u.login_name)
            }
        })
        .unwrap_or_else(|| "-".into());
    format!(
        "{:<15} {:<20} {:<12} {:<7} {}\n",
        ip,
        ps.name(),
        owner,
        ps.os,
        state
    )
}

fn peer_state(ps: &PeerStatus) -> String {
    let mut parts: Vec<String> = Vec::new();
    if ps.exit_node {
        parts.push("exit node".into());
    }
    if ps.expired {
        parts.push("expired".into());
    }
    if !ps.online {
        parts.push("offline".into());
    } else if ps.active {
        if !ps.cur_addr.is_empty() {
            parts.push(format!("active; direct {}", ps.cur_addr));
        } else if !ps.relay.is_empty() {
            parts.push(format!("active; relay {:?}", ps.relay));
        } else {
            parts.push("active".into());
        }
    } else {
        parts.push("idle".into());
    }
    parts.join("; ")
}

/// Renders a successful ping like the Go CLI:
/// `pong from node2 (100.64.0.2) via 127.0.0.1:41642 in 2ms`.
pub fn pong(pr: &PingResult) -> String {
    let via = if !pr.endpoint.is_empty() {
        pr.endpoint.clone()
    } else if !pr.derp_region_code.is_empty() {
        format!("DERP({})", pr.derp_region_code)
    } else if !pr.peer_relay.is_empty() {
        format!("peer-relay({})", pr.peer_relay)
    } else {
        "?".into()
    };
    let ms = pr.latency_seconds * 1000.0;
    format!(
        "pong from {} ({}) via {} in {:.0}ms",
        pr.node_name, pr.node_ip, via, ms
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_status() -> Status {
        serde_json::from_str(include_str!("../../ts-types/tests/fixtures/status.json")).unwrap()
    }

    #[test]
    fn renders_fixture_status() {
        let text = status(&fixture_status());
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            lines[0].starts_with("100.64.0.1"),
            "self first: {}",
            lines[0]
        );
        assert!(lines[0].contains("node1") && lines[0].contains("interop@"));
        assert!(lines[1].starts_with("100.64.0.2"));
        assert!(
            lines[1].contains("active; direct 127.0.0.1:41642"),
            "peer link state: {}",
            lines[1]
        );
        assert!(text.contains("# Health check:"));
    }

    #[test]
    fn renders_pong() {
        let pr: PingResult =
            serde_json::from_str(include_str!("../../ts-types/tests/fixtures/ping.json")).unwrap();
        assert_eq!(
            pong(&pr),
            "pong from node2 (100.64.0.2) via 127.0.0.1:41642 in 0ms"
        );
    }

    #[test]
    fn offline_and_relay_states() {
        let mut ps = PeerStatus {
            online: false,
            ..Default::default()
        };
        assert_eq!(peer_state(&ps), "offline");
        ps.online = true;
        ps.active = true;
        ps.relay = "headscale".into();
        assert_eq!(peer_state(&ps), "active; relay \"headscale\"");
        ps.cur_addr = "10.0.0.1:1".into();
        assert_eq!(peer_state(&ps), "active; direct 10.0.0.1:1");
    }
}

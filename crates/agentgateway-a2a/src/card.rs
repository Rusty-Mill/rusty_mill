//! Merging the agent cards of the agents behind a route.
//!
//! # Why the URL matters most
//!
//! An agent card advertises the address clients should call. An agent behind a
//! gateway advertises *its own* address, so a client that reads the card
//! verbatim goes straight around the gateway — past its authentication, its
//! rate limits and its audit trail. Rewriting the URL to the gateway's is the
//! single thing that makes a proxied card usable, which is why
//! [`agentgateway_config::AgentCardPolicy::url`] is required rather than
//! optional.
//!
//! # Parsing is lenient on purpose
//!
//! [`rusty_a2a`]'s types are transliterated field-for-field from the normative
//! proto, with required fields enforced — correct for an agent, awkward for a
//! gateway. One upstream agent serving a card that omits a required field
//! would otherwise break discovery for every other agent behind the same
//! route. So a card that will not parse is skipped and reported, and the merge
//! proceeds with the rest.
//!
//! # What a merged card does not promise
//!
//! The union of skills describes what is reachable behind the route, not a
//! routing table. This gateway load-balances across backends by weight and
//! does **not** route by skill, so a client picking a skill from the merged
//! card is not guaranteed to reach the agent that offers it. With one backend
//! — the common case of a gateway fronting a single agent — the question does
//! not arise, and the card is that agent's with its URL corrected.

use agentgateway_config::AgentCardPolicy;
use rusty_a2a::types::AgentCard;
use serde_json::Value;

/// A card that could not be used, and why.
#[derive(Debug, Clone)]
pub struct Rejected {
    /// The agent it came from.
    pub source: String,
    /// What went wrong.
    pub reason: String,
}

/// The result of merging what the backends served.
#[derive(Debug, Clone)]
pub struct Merged {
    /// The card to serve.
    pub card: AgentCard,
    /// Cards that could not be parsed.
    pub rejected: Vec<Rejected>,
    /// Skill ids that appeared on more than one agent.
    pub collisions: Vec<String>,
}

/// Parse a card leniently, reporting rather than failing.
pub fn parse(source: &str, body: &[u8]) -> Result<AgentCard, Rejected> {
    serde_json::from_slice(body).map_err(|err| Rejected {
        source: source.to_string(),
        reason: err.to_string(),
    })
}

/// Merge the agents' cards into the one the gateway serves.
///
/// `cards` is `(agent, card)` pairs in backend order.
pub fn merge(cards: Vec<(String, AgentCard)>, policy: &AgentCardPolicy) -> Option<Merged> {
    let (_, first) = cards.first()?;
    let mut card = first.clone();

    // The whole point: clients must be told to call us, not the agent.
    for interface in &mut card.supported_interfaces {
        interface.url = policy.url.clone();
    }

    if let Some(name) = &policy.name {
        card.name = name.clone();
    }
    if let Some(description) = &policy.description {
        card.description = description.clone();
    }

    let mut collisions = Vec::new();

    for (agent, other) in cards.iter().skip(1) {
        for skill in &other.skills {
            if card.skills.iter().any(|existing| existing.id == skill.id) {
                // Reported rather than renamed: unlike an MCP tool name, a
                // skill id is descriptive and not what a caller invokes, so
                // silently qualifying it would misrepresent the agent.
                collisions.push(format!("`{}` is offered by {agent} and another agent", skill.id));
                continue;
            }
            card.skills.push(skill.clone());
        }

        // Capabilities are intersected, not unioned. Advertising streaming
        // because one agent supports it sends a streaming client to an agent
        // that cannot, and the failure lands on the client.
        intersect_capabilities(&mut card, other);
    }

    Some(Merged {
        card,
        rejected: Vec::new(),
        collisions,
    })
}

/// Keep only the capabilities every agent supports.
fn intersect_capabilities(card: &mut AgentCard, other: &AgentCard) {
    let a = serde_json::to_value(&card.capabilities).unwrap_or(Value::Null);
    let b = serde_json::to_value(&other.capabilities).unwrap_or(Value::Null);

    let (Some(a), Some(b)) = (a.as_object(), b.as_object()) else {
        return;
    };

    let mut merged = serde_json::Map::new();
    for (key, value) in a {
        match (value.as_bool(), b.get(key).and_then(Value::as_bool)) {
            (Some(ours), Some(theirs)) => {
                merged.insert(key.clone(), Value::Bool(ours && theirs));
            }
            // A capability only one side describes, or one that is not a
            // simple flag, is left as ours rather than guessed at.
            _ => {
                merged.insert(key.clone(), value.clone());
            }
        }
    }

    if let Ok(capabilities) = serde_json::from_value(Value::Object(merged)) {
        card.capabilities = capabilities;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn card(name: &str, url: &str, skills: &[&str], streaming: bool) -> AgentCard {
        let skills: Vec<Value> = skills
            .iter()
            .map(|id| json!({"id": id, "name": id, "description": "d", "tags": []}))
            .collect();
        serde_json::from_value(json!({
            "name": name,
            "description": "an agent",
            "version": "1.0",
            "protocolVersion": "1.0",
            "supportedInterfaces": [
                {"protocolBinding": "JSONRPC", "protocolVersion": "1.0", "url": url}
            ],
            "capabilities": {"streaming": streaming},
            "defaultInputModes": ["text/plain"],
            "defaultOutputModes": ["text/plain"],
            "skills": skills,
        }))
        .expect("test card should parse")
    }

    fn policy(url: &str) -> AgentCardPolicy {
        AgentCardPolicy {
            url: url.to_string(),
            name: None,
            description: None,
        }
    }

    #[test]
    fn the_url_is_rewritten_to_the_gateways() {
        // Without this a client reads the agent's own address and goes around
        // the gateway entirely.
        let merged = merge(
            vec![("a".into(), card("Agent", "http://agent:9000", &["echo"], true))],
            &policy("https://gateway.example.com/a2a"),
        )
        .expect("should merge");

        assert_eq!(
            merged.card.supported_interfaces[0].url,
            "https://gateway.example.com/a2a"
        );
    }

    #[test]
    fn a_single_agent_card_is_otherwise_a_faithful_passthrough() {
        let original = card("Agent", "http://agent:9000", &["echo", "sum"], true);
        let merged = merge(
            vec![("a".into(), original.clone())],
            &policy("https://gw/a2a"),
        )
        .expect("should merge");

        assert_eq!(merged.card.name, original.name);
        assert_eq!(merged.card.skills.len(), 2);
        assert!(merged.collisions.is_empty());
    }

    #[test]
    fn skills_from_several_agents_are_unioned() {
        let merged = merge(
            vec![
                ("a".into(), card("A", "http://a", &["echo"], true)),
                ("b".into(), card("B", "http://b", &["summarise"], true)),
            ],
            &policy("https://gw/a2a"),
        )
        .expect("should merge");

        let ids: Vec<&str> = merged.card.skills.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["echo", "summarise"]);
    }

    #[test]
    fn a_duplicated_skill_is_reported_rather_than_renamed() {
        // A skill id is descriptive, not what a caller invokes, so qualifying
        // it would misrepresent the agent rather than disambiguate anything.
        let merged = merge(
            vec![
                ("a".into(), card("A", "http://a", &["echo"], true)),
                ("b".into(), card("B", "http://b", &["echo"], true)),
            ],
            &policy("https://gw/a2a"),
        )
        .expect("should merge");

        assert_eq!(merged.card.skills.len(), 1, "listed once");
        assert_eq!(merged.collisions.len(), 1);
        assert!(merged.collisions[0].contains("echo"));
    }

    #[test]
    fn capabilities_are_intersected_not_unioned() {
        // Advertising streaming because one agent has it sends a streaming
        // client to an agent that cannot, and the failure lands on the client.
        let merged = merge(
            vec![
                ("a".into(), card("A", "http://a", &["x"], true)),
                ("b".into(), card("B", "http://b", &["y"], false)),
            ],
            &policy("https://gw/a2a"),
        )
        .expect("should merge");

        let capabilities =
            serde_json::to_value(&merged.card.capabilities).expect("should serialize");
        assert_eq!(
            capabilities["streaming"], false,
            "only what every agent supports may be advertised"
        );
    }

    #[test]
    fn the_name_and_description_can_be_overridden() {
        let merged = merge(
            vec![("a".into(), card("Agent", "http://a", &["x"], true))],
            &AgentCardPolicy {
                url: "https://gw/a2a".into(),
                name: Some("The Gateway".into()),
                description: Some("Everything behind it".into()),
            },
        )
        .expect("should merge");

        assert_eq!(merged.card.name, "The Gateway");
        assert_eq!(merged.card.description, "Everything behind it");
    }

    #[test]
    fn no_cards_means_nothing_to_serve() {
        assert!(merge(Vec::new(), &policy("https://gw/a2a")).is_none());
    }

    #[test]
    fn a_malformed_card_is_reported_rather_than_fatal() {
        // One non-conformant agent must not break discovery for the rest.
        let rejected = parse("agent-b", b"{\"name\": \"missing everything else\"}")
            .expect_err("should not parse");
        assert_eq!(rejected.source, "agent-b");
        assert!(!rejected.reason.is_empty());
    }
}

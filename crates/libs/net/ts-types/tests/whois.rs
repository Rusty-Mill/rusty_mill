//! `GET /localapi/v0/whois` decoding: a user's node, a tagged node, an unknown peer.

use ts_types::tailcfg::WhoIsResponse;

#[test]
fn a_users_node_has_a_login_and_no_tags() {
    let json = r#"{"Node":{"ID":7,"Name":"lap.tail1234.ts.net.","Tags":null},
        "UserProfile":{"ID":3,"LoginName":"nano@example.com","DisplayName":"Nano"},
        "CapMap":{"x":[1]}}"#;
    let who: WhoIsResponse = serde_json::from_str(json).unwrap();
    let node = who.node.unwrap();
    assert_eq!(node.name, "lap.tail1234.ts.net.");
    assert!(node.tags.is_empty());
    assert_eq!(who.user_profile.unwrap().login_name, "nano@example.com");
}

#[test]
fn a_tagged_node_carries_its_tags() {
    let json = r#"{"Node":{"Name":"srv.tail1234.ts.net.","Tags":["tag:server"]},"UserProfile":{"LoginName":"tagged-devices"}}"#;
    let who: WhoIsResponse = serde_json::from_str(json).unwrap();
    assert_eq!(who.node.unwrap().tags, ["tag:server"]);
}

#[test]
fn an_unknown_peer_decodes_to_nothing() {
    let who: WhoIsResponse = serde_json::from_str("{}").unwrap();
    assert!(who.node.is_none() && who.user_profile.is_none());
}

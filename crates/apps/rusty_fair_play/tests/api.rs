//! The HTTP API through the sans-IO router: no sockets.

use rusty_fair_play::api::{Api, Request};
use rusty_fair_play::service::Service;
use rusty_fair_play_domain::deck_card_id;
use rusty_http::Method;
use rusty_json::Value;

const TOKEN: &str = "test-token-0123456789";

struct Harness {
    api: Api,
}

impl Harness {
    fn open(dir: &std::path::Path, token: Option<&str>) -> Self {
        let mut service = Service::open(dir).unwrap();
        service.seed_deck().unwrap();
        Self {
            api: Api::new(service, token.map(String::from)).unwrap(),
        }
    }

    fn send(
        &mut self,
        method: Method,
        target: &str,
        body: &str,
        auth: Option<&str>,
    ) -> (u16, Value) {
        self.send_with(method, target, body, auth, None)
    }

    fn send_with(
        &mut self,
        method: Method,
        target: &str,
        body: &str,
        auth: Option<&str>,
        if_match: Option<&str>,
    ) -> (u16, Value) {
        let header = auth.map(|t| format!("Bearer {t}"));
        let request = Request {
            method: &method,
            target,
            authorization: header.as_deref(),
            if_match,
            body: body.as_bytes(),
        };
        let response = self.api.handle(&request);
        let value = if response.body.is_empty() {
            Value::Null
        } else {
            rusty_json::from_slice(&response.body).expect("responses are JSON")
        };
        (response.status.as_u16(), value)
    }

    fn call(&mut self, method: Method, target: &str, body: &str) -> (u16, Value) {
        self.send(method, target, body, Some(TOKEN))
    }

    fn person(&mut self, name: &str) -> String {
        let (status, v) = self.call(
            Method::Post,
            "/api/v1/people",
            &format!(r#"{{"name":"{name}"}}"#),
        );
        assert_eq!(status, 201, "{v:?}");
        v["id"].as_str().unwrap().to_string()
    }
}

fn harness() -> (tempfile::TempDir, Harness) {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::open(dir.path(), Some(TOKEN));
    (dir, h)
}

fn card(n: u16) -> String {
    deck_card_id(n).to_string()
}

#[test]
fn health_is_open_and_the_token_gates_the_api_when_set() {
    let (_d, mut h) = harness();
    assert_eq!(h.send(Method::Get, "/health", "", None).0, 200);
    assert_eq!(h.send(Method::Get, "/api/v1/snapshot", "", None).0, 401);
    assert_eq!(
        h.send(
            Method::Get,
            "/api/v1/snapshot",
            "",
            Some("wrong-token-0123456789")
        )
        .0,
        401
    );
    assert_eq!(
        h.send(Method::Get, "/api/v1/snapshot", "", Some(TOKEN)).0,
        200
    );
    assert_eq!(h.call(Method::Get, "/api/v2/snapshot", "").0, 404);
    let dir = tempfile::tempdir().unwrap();
    assert!(
        Api::new(Service::open(dir.path()).unwrap(), Some("short".into())).is_err(),
        "a guessable token is refused"
    );
}

#[test]
fn without_a_token_the_api_is_open() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::open(dir.path(), None);
    let (status, v) = h.send(Method::Get, "/api/v1/snapshot", "", None);
    assert_eq!(status, 200);
    assert_eq!(v["cards"].as_array().unwrap().len(), 100);
}

#[test]
fn snapshot_carries_the_deck_with_state_and_seed_is_idempotent() {
    let (_d, mut h) = harness();
    let (_, v) = h.call(Method::Get, "/api/v1/snapshot", "");
    let cards = v["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 100);
    assert!(v["people"].as_array().unwrap().is_empty());
    let dinner = cards
        .iter()
        .find(|c| c["number"].as_i64() == Some(17))
        .unwrap();
    assert_eq!(dinner["name"].as_str(), Some("Meals (Weekday Dinner)"));
    assert_eq!(dinner["suit"].as_str(), Some("Home"));
    assert_eq!(dinner["state"].as_str(), Some("original"));
    assert_eq!(dinner["origin"].as_str(), Some("deck"));
    assert!(dinner["ownerId"].is_null());
    assert!(dinner["parentCardId"].is_null());
    assert_eq!(dinner["minimumStandardOfCare"].as_array().unwrap().len(), 3);
    let (status, v) = h.call(Method::Post, "/api/v1/seed", "");
    assert_eq!(status, 200);
    assert_eq!(v["cards"]["created"].as_i64(), Some(0));
    assert_eq!(v["cards"]["existing"].as_i64(), Some(100));
}

#[test]
fn deal_split_edit_reset_and_custom_cards_over_the_api() {
    let (_d, mut h) = harness();
    let ada = h.person("Ada");
    let bob = h.person("Bob");
    assert_eq!(
        h.call(Method::Post, "/api/v1/people", r#"{"name":"Ada"}"#)
            .0,
        409
    );
    assert_eq!(
        h.call(Method::Post, "/api/v1/people", r#"{"name":" "}"#).0,
        422
    );
    assert_eq!(
        h.call(Method::Post, "/api/v1/people", r#"{"nam":"x"}"#).0,
        400,
        "unknown field"
    );
    let (status, v) = h.call(
        Method::Patch,
        &format!("/api/v1/people/{bob}"),
        r#"{"name":"Robert"}"#,
    );
    assert_eq!(
        (status, v["name"].as_str(), v["player"].as_i64()),
        (200, Some("Robert"), Some(2))
    );

    // Deal: ownership never changes the state.
    let cleaning = card(2);
    let (status, v) = h.call(
        Method::Patch,
        &format!("/api/v1/cards/{cleaning}"),
        &format!(r#"{{"ownerId":"{ada}"}}"#),
    );
    assert_eq!(
        (status, v["ownerId"].as_str(), v["state"].as_str()),
        (200, Some(ada.as_str()), Some("original"))
    );
    assert_eq!(
        h.call(
            Method::Patch,
            &format!("/api/v1/cards/{cleaning}"),
            r#"{"ownerId":"00000000-0000-0000-0000-000000000009"}"#
        )
        .0,
        422,
        "an unknown owner"
    );
    assert_eq!(
        h.call(
            Method::Patch,
            &format!("/api/v1/cards/{cleaning}"),
            r#"{"bogus":1}"#
        )
        .0,
        400
    );
    assert_eq!(
        h.call(Method::Patch, "/api/v1/cards/not-a-uuid", r#"{}"#).0,
        400
    );
    assert_eq!(
        h.call(
            Method::Patch,
            "/api/v1/cards/00000000-0000-0000-0000-000000000009",
            r#"{}"#
        )
        .0,
        404
    );

    // Split: children are custom, parent keeps its state, cycle refused.
    let (status, v) = h.call(
        Method::Post,
        &format!("/api/v1/cards/{cleaning}/split"),
        &format!(r#"{{"children":[{{"name":"Bathrooms","ownerId":"{ada}"}},{{"name":"Floors","ownerId":"{bob}","minimumStandardOfCare":["Mopped weekly"]}}],"ownerId":null}}"#),
    );
    assert_eq!(status, 201, "{v:?}");
    let children = v["children"].as_array().unwrap();
    assert_eq!(children.len(), 2);
    assert_eq!(children[0]["state"].as_str(), Some("custom"));
    assert_eq!(children[1]["position"].as_i64(), Some(1));
    assert_eq!(children[1]["suit"].as_str(), Some("Home"));
    assert!(
        v["parent"]["ownerId"].is_null(),
        "the parent was handed back"
    );
    assert_eq!(v["parent"]["state"].as_str(), Some("original"));
    let floors = children[1]["id"].as_str().unwrap().to_string();
    assert_eq!(
        h.call(
            Method::Post,
            &format!("/api/v1/cards/{cleaning}/split"),
            r#"{"children":[]}"#
        )
        .0,
        422
    );
    assert_eq!(
        h.call(
            Method::Patch,
            &format!("/api/v1/cards/{cleaning}"),
            &format!(r#"{{"parentCardId":"{floors}"}}"#)
        )
        .0,
        422,
        "a cycle is refused"
    );

    // Edit one of the six: edited; the diff names it; reset restores it.
    let (status, v) = h.call(
        Method::Patch,
        &format!("/api/v1/cards/{cleaning}"),
        r#"{"execution":"Our way","notes":"n"}"#,
    );
    assert_eq!((status, v["state"].as_str()), (200, Some("edited")));
    let (_, v) = h.call(
        Method::Get,
        &format!("/api/v1/cards/{cleaning}/baseline"),
        "",
    );
    assert_eq!(v["diff"].as_array().unwrap().len(), 1);
    assert_eq!(v["diff"][0]["field"].as_str(), Some("execution"));
    assert_eq!(v["baseline"]["number"].as_i64(), Some(2));
    let (status, v) = h.call(Method::Post, &format!("/api/v1/cards/{cleaning}/reset"), "");
    assert_eq!(
        (status, v["state"].as_str(), v["notes"].as_str()),
        (200, Some("original"), Some("n"))
    );
    assert_eq!(
        h.call(Method::Post, &format!("/api/v1/cards/{floors}/reset"), "")
            .0,
        422,
        "a custom card has no baseline"
    );
    let (_, v) = h.call(Method::Get, &format!("/api/v1/cards/{floors}/baseline"), "");
    assert!(v["baseline"].is_null());

    // Position: one slot write.
    assert_eq!(
        h.call(
            Method::Put,
            &format!("/api/v1/cards/{floors}/position"),
            r#"{"position":0}"#
        )
        .0,
        204
    );
    let (_, v) = h.call(Method::Get, &format!("/api/v1/cards/{floors}"), "");
    assert_eq!(v["position"].as_i64(), Some(0));

    // A custom card, with and without a client id; a bad suit.
    let (status, v) = h.call(
        Method::Post,
        "/api/v1/cards",
        &format!(r#"{{"name":"Dog walking","suit":"Out","ownerId":"{bob}"}}"#),
    );
    assert_eq!(
        (status, v["state"].as_str(), v["origin"].as_str()),
        (201, Some("custom"), Some("family"))
    );
    assert!(v["number"].is_null());
    let (status, v) = h.call(
        Method::Post,
        "/api/v1/cards",
        r#"{"id":"11111111-1111-4111-8111-111111111111","name":"Plants","suit":"Home"}"#,
    );
    assert_eq!(
        (status, v["id"].as_str()),
        (201, Some("11111111-1111-4111-8111-111111111111"))
    );
    assert_eq!(
        h.call(
            Method::Post,
            "/api/v1/cards",
            r#"{"id":"11111111-1111-4111-8111-111111111111","name":"Plants","suit":"Home"}"#
        )
        .0,
        409
    );
    assert_eq!(
        h.call(
            Method::Post,
            "/api/v1/cards",
            r#"{"name":"x","suit":"Spades"}"#
        )
        .0,
        422
    );

    let (_, v) = h.call(Method::Get, "/api/v1/snapshot", "");
    assert_eq!(v["cards"].as_array().unwrap().len(), 104);
    assert_eq!(v["people"].as_array().unwrap().len(), 2);
}

#[test]
fn delete_unsplit_reorder_and_if_match_over_the_api() {
    let (_d, mut h) = harness();
    let ada = h.person("Ada");
    let cleaning = card(2);
    let (status, v) = h.call(
        Method::Post,
        &format!("/api/v1/cards/{cleaning}/split"),
        r#"{"children":[{"name":"Bathrooms"},{"name":"Floors"},{"name":"Windows"}]}"#,
    );
    assert_eq!(status, 201, "{v:?}");
    let ids: Vec<String> = v["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect();
    let etag = v["parent"]["etag"].as_str().unwrap().to_string();
    assert_eq!(etag.len(), 16, "eight bytes of SHA-256, hex");

    // Atomic reorder: exact set or 422; the children come back in order.
    let order = format!("/api/v1/cards/{cleaning}/children/order");
    let (status, v) = h.call(
        Method::Put,
        &order,
        &format!(r#"{{"ids":["{}","{}","{}"]}}"#, ids[2], ids[0], ids[1]),
    );
    assert_eq!(status, 200, "{v:?}");
    let back: Vec<&str> = v["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        back,
        vec![ids[2].as_str(), ids[0].as_str(), ids[1].as_str()]
    );
    assert_eq!(v["cards"][0]["position"].as_i64(), Some(0));
    assert_eq!(
        h.call(Method::Put, &order, &format!(r#"{{"ids":["{}"]}}"#, ids[0]))
            .0,
        422
    );
    assert_eq!(
        h.call(Method::Put, &order, r#"{"ids":["not-a-uuid"]}"#).0,
        400
    );

    // If-Match: a stale tag is a 412 carrying the current card.
    let (status, v) = h.send_with(
        Method::Patch,
        &format!("/api/v1/cards/{cleaning}"),
        r#"{"notes":"x"}"#,
        Some(TOKEN),
        Some("\"0000000000000000\""),
    );
    assert_eq!(status, 412, "{v:?}");
    assert_eq!(v["error"]["code"].as_str(), Some("precondition_failed"));
    assert_eq!(v["current"]["id"].as_str(), Some(cleaning.as_str()));
    assert_eq!(v["current"]["etag"].as_str(), Some(etag.as_str()));
    let (status, v) = h.send_with(
        Method::Patch,
        &format!("/api/v1/cards/{cleaning}"),
        r#"{"notes":"x"}"#,
        Some(TOKEN),
        Some(&format!("\"{etag}\"")),
    );
    assert_eq!(status, 200, "{v:?}");
    assert_ne!(v["etag"].as_str(), Some(etag.as_str()), "the tag moved");
    let (status, _) = h.send_with(
        Method::Patch,
        &format!("/api/v1/cards/{cleaning}"),
        r#"{"notes":"y"}"#,
        Some(TOKEN),
        Some("*"),
    );
    assert_eq!(status, 200, "a wildcard always matches");

    // Delete: a leaf goes, a parent is refused until unsplit.
    assert_eq!(
        h.call(Method::Delete, &format!("/api/v1/cards/{cleaning}"), "")
            .0,
        409
    );
    assert_eq!(
        h.call(Method::Delete, &format!("/api/v1/cards/{}", ids[1]), "")
            .0,
        204
    );
    assert_eq!(
        h.call(Method::Get, &format!("/api/v1/cards/{}", ids[1]), "")
            .0,
        404
    );
    let (status, v) = h.call(
        Method::Post,
        &format!("/api/v1/cards/{cleaning}/unsplit"),
        "",
    );
    assert_eq!(status, 200, "{v:?}");
    assert_eq!(v["deleted"].as_array().unwrap().len(), 2);
    assert_eq!(
        h.call(Method::Delete, &format!("/api/v1/cards/{cleaning}"), "")
            .0,
        204,
        "a deck card is deletable once it is a leaf"
    );

    // A person holding a card stays; one holding none goes.
    let laundry = card(3);
    h.call(
        Method::Patch,
        &format!("/api/v1/cards/{laundry}"),
        &format!(r#"{{"ownerId":"{ada}"}}"#),
    );
    assert_eq!(
        h.call(Method::Delete, &format!("/api/v1/people/{ada}"), "")
            .0,
        409
    );
    h.call(
        Method::Patch,
        &format!("/api/v1/cards/{laundry}"),
        r#"{"ownerId":null}"#,
    );
    assert_eq!(
        h.call(Method::Delete, &format!("/api/v1/people/{ada}"), "")
            .0,
        204
    );
    assert_eq!(
        h.call(Method::Delete, &format!("/api/v1/people/{ada}"), "")
            .0,
        404
    );
}

#[test]
fn subtree_writes_are_guarded_by_the_tree_etag() {
    let (_d, mut h) = harness();
    let cleaning = card(2);
    let (status, v) = h.call(
        Method::Post,
        &format!("/api/v1/cards/{cleaning}/split"),
        r#"{"children":[{"name":"Bathrooms"},{"name":"Floors"}]}"#,
    );
    assert_eq!(status, 201, "{v:?}");
    let (a, b) = (
        v["children"][0]["id"].as_str().unwrap().to_string(),
        v["children"][1]["id"].as_str().unwrap().to_string(),
    );
    let (_, v) = h.call(Method::Get, &format!("/api/v1/cards/{cleaning}"), "");
    let (etag, tree) = (
        v["etag"].as_str().unwrap().to_string(),
        v["treeEtag"].as_str().unwrap().to_string(),
    );

    // Another client edits a child: the parent's own tag is unchanged,
    // its tree tag is not.
    let (status, _) = h.call(
        Method::Patch,
        &format!("/api/v1/cards/{a}"),
        r#"{"notes":"edited elsewhere"}"#,
    );
    assert_eq!(status, 200);
    let (_, v) = h.call(Method::Get, &format!("/api/v1/cards/{cleaning}"), "");
    assert_eq!(v["etag"].as_str(), Some(etag.as_str()));
    assert_ne!(v["treeEtag"].as_str(), Some(tree.as_str()));

    // So a reorder or unsplit based on the old read is refused, and
    // nothing changed.
    let stale = format!("\"{tree}\"");
    let order = format!("/api/v1/cards/{cleaning}/children/order");
    let (status, v) = h.send_with(
        Method::Put,
        &order,
        &format!(r#"{{"ids":["{b}","{a}"]}}"#),
        Some(TOKEN),
        Some(&stale),
    );
    assert_eq!(status, 412, "{v:?}");
    assert_ne!(v["current"]["treeEtag"].as_str(), Some(tree.as_str()));
    let (status, _) = h.send_with(
        Method::Post,
        &format!("/api/v1/cards/{cleaning}/unsplit"),
        "",
        Some(TOKEN),
        Some(&stale),
    );
    assert_eq!(status, 412);
    assert_eq!(
        h.call(Method::Get, &format!("/api/v1/cards/{a}"), "").0,
        200,
        "the unsplit did not run"
    );

    // The fresh tag goes through; the card's own tag does not guard a
    // subtree write once a descendant has moved on.
    let fresh = v["current"]["treeEtag"].as_str().unwrap().to_string();
    let (status, _) = h.send_with(
        Method::Put,
        &order,
        &format!(r#"{{"ids":["{b}","{a}"]}}"#),
        Some(TOKEN),
        Some(&format!("\"{fresh}\"")),
    );
    assert_eq!(status, 200);
    let (status, _) = h.send_with(
        Method::Post,
        &format!("/api/v1/cards/{cleaning}/unsplit"),
        "",
        Some(TOKEN),
        Some(&format!("\"{etag}\"")),
    );
    assert_eq!(status, 412, "the reorder moved the tree tag again");
}

#[test]
fn a_family_chooses_its_deck_over_the_api() {
    let (_d, mut h) = harness();
    let ada = h.person("Ada");
    let dishes = card(3);
    let path = format!("/api/v1/cards/{dishes}");
    let (_, v) = h.call(Method::Get, &path, "");
    assert_eq!(v["inPlay"].as_bool(), Some(true));
    let etag = v["etag"].as_str().unwrap().to_string();
    h.call(Method::Patch, &path, &format!(r#"{{"ownerId":"{ada}"}}"#));

    let (status, v) = h.call(Method::Patch, &path, r#"{"inPlay":false}"#);
    assert_eq!(status, 200, "{v:?}");
    assert_eq!(v["inPlay"].as_bool(), Some(false));
    assert!(v["ownerId"].is_null(), "taken back from Ada");
    assert_ne!(v["etag"].as_str(), Some(etag.as_str()));
    let (_, snap) = h.call(Method::Get, "/api/v1/snapshot", "");
    let out: Vec<_> = snap["cards"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["inPlay"] == false)
        .collect();
    assert_eq!(out.len(), 1);

    // Out of play: 422 to deal or split; a split parent is 409.
    let deal = format!(r#"{{"ownerId":"{ada}"}}"#);
    assert_eq!(h.call(Method::Patch, &path, &deal).0, 422);
    assert_eq!(
        h.call(
            Method::Post,
            &format!("{path}/split"),
            r#"{"children":[{"name":"x"}]}"#
        )
        .0,
        422
    );
    let cleaning = card(2);
    h.call(
        Method::Post,
        &format!("/api/v1/cards/{cleaning}/split"),
        r#"{"children":[{"name":"A"}]}"#,
    );
    assert_eq!(
        h.call(
            Method::Patch,
            &format!("/api/v1/cards/{cleaning}"),
            r#"{"inPlay":false}"#
        )
        .0,
        409
    );
    assert_eq!(h.call(Method::Patch, &path, r#"{"inPlay":"no"}"#).0, 400);

    let (status, v) = h.call(Method::Patch, &path, r#"{"inPlay":true}"#);
    assert_eq!((status, v["inPlay"].as_bool()), (200, Some(true)));
    assert_eq!(h.call(Method::Patch, &path, &deal).0, 200);
}

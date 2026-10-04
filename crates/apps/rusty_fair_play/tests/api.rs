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
        let header = auth.map(|t| format!("Bearer {t}"));
        let request = Request {
            method: &method,
            target,
            authorization: header.as_deref(),
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

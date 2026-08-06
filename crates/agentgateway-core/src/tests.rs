//! Routing and CORS tests.

use agentgateway_config::Config;
use http::{Method, Request, header};

use crate::{CorsDecision, CorsMatcher, HostnamePattern, Router};

fn router(yaml: &str) -> Router {
    let config = Config::from_yaml(yaml).expect("config should parse");
    config.validate().expect("config should validate");
    Router::build(&config).expect("router should build")
}

fn get(path: &str) -> Request<()> {
    Request::builder()
        .uri(path)
        .body(())
        .expect("request should build")
}

fn named(selection: Option<crate::Selection<'_>>) -> Option<String> {
    selection.map(|s| {
        s.route
            .name
            .clone()
            .expect("routes in these tests are named")
    })
}

const PRECEDENCE_CONFIG: &str = r#"
binds:
  - port: 8080
    listeners:
      - routes:
          - name: catch-all
            matches:
              - path:
                  pathPrefix: /
            backends: [{host: "a:80"}]
          - name: short-prefix
            matches:
              - path:
                  pathPrefix: /api
            backends: [{host: "b:80"}]
          - name: long-prefix
            matches:
              - path:
                  pathPrefix: /api/v1
            backends: [{host: "c:80"}]
          - name: exact
            matches:
              - path:
                  exact: /api/v1/health
            backends: [{host: "d:80"}]
"#;

#[test]
fn exact_beats_prefix_and_longer_prefix_beats_shorter() {
    let router = router(PRECEDENCE_CONFIG);

    // Declaration order in the config is deliberately the reverse of
    // specificity, so passing this proves precedence is not just file order.
    assert_eq!(
        named(router.select(8080, &get("/api/v1/health"))).as_deref(),
        Some("exact")
    );
    assert_eq!(
        named(router.select(8080, &get("/api/v1/tools"))).as_deref(),
        Some("long-prefix")
    );
    assert_eq!(
        named(router.select(8080, &get("/api/other"))).as_deref(),
        Some("short-prefix")
    );
    assert_eq!(
        named(router.select(8080, &get("/unrelated"))).as_deref(),
        Some("catch-all")
    );
}

#[test]
fn prefix_matching_respects_segment_boundaries() {
    let router = router(
        r#"
binds:
  - port: 8080
    listeners:
      - routes:
          - name: admin
            matches:
              - path:
                  pathPrefix: /admin
            backends: [{host: "a:80"}]
"#,
    );

    assert_eq!(named(router.select(8080, &get("/admin"))).as_deref(), Some("admin"));
    assert_eq!(
        named(router.select(8080, &get("/admin/users"))).as_deref(),
        Some("admin")
    );
    // The whole point: `/admin` must not capture `/admin-public`.
    assert_eq!(named(router.select(8080, &get("/admin-public"))), None);
    assert_eq!(named(router.select(8080, &get("/administrator"))), None);
}

#[test]
fn a_route_with_more_predicates_wins_a_path_tie() {
    let router = router(
        r#"
binds:
  - port: 8080
    listeners:
      - routes:
          - name: plain
            matches:
              - path:
                  pathPrefix: /api
            backends: [{host: "a:80"}]
          - name: with-header
            matches:
              - path:
                  pathPrefix: /api
                headers:
                  - name: x-canary
                    exact: "true"
            backends: [{host: "b:80"}]
"#,
    );

    let canary = Request::builder()
        .uri("/api/thing")
        .header("x-canary", "true")
        .body(())
        .expect("request should build");
    assert_eq!(named(router.select(8080, &canary)).as_deref(), Some("with-header"));
    assert_eq!(
        named(router.select(8080, &get("/api/thing"))).as_deref(),
        Some("plain"),
        "a request without the header falls through to the less specific route"
    );
}

#[test]
fn method_and_query_predicates_are_enforced() {
    let router = router(
        r#"
binds:
  - port: 8080
    listeners:
      - routes:
          - name: post-only
            matches:
              - method: POST
                query:
                  - name: mode
                    exact: "fast lane"
            backends: [{host: "a:80"}]
"#,
    );

    // `+` and `%20` both decode to a space, so both must match the literal in
    // the config file.
    for query in ["mode=fast+lane", "mode=fast%20lane"] {
        let request = Request::builder()
            .method(Method::POST)
            .uri(format!("/anything?{query}"))
            .body(())
            .expect("request should build");
        assert_eq!(
            named(router.select(8080, &request)).as_deref(),
            Some("post-only"),
            "query {query} should match"
        );
    }

    let wrong_method = Request::builder()
        .method(Method::GET)
        .uri("/anything?mode=fast+lane")
        .body(())
        .expect("request should build");
    assert_eq!(named(router.select(8080, &wrong_method)), None);
}

#[test]
fn hostname_selects_between_listeners_and_routes() {
    let router = router(
        r#"
binds:
  - port: 8080
    listeners:
      - name: wildcard
        hostname: "*.example.com"
        routes:
          - name: wildcard-route
            backends: [{host: "a:80"}]
      - name: exact
        hostname: api.example.com
        routes:
          - name: exact-route
            backends: [{host: "b:80"}]
"#,
    );

    let request = |host: &str| {
        Request::builder()
            .uri("/")
            .header(header::HOST, host)
            .body(())
            .expect("request should build")
    };

    assert_eq!(
        named(router.select(8080, &request("api.example.com"))).as_deref(),
        Some("exact-route"),
        "the more specific listener wins even though the wildcard also matches"
    );
    assert_eq!(
        named(router.select(8080, &request("web.example.com"))).as_deref(),
        Some("wildcard-route")
    );
    assert_eq!(
        named(router.select(8080, &request("other.org"))),
        None,
        "no listener claims this hostname"
    );
}

#[test]
fn hostname_patterns_match_one_label_only() {
    let wildcard = HostnamePattern::parse("*.example.com");
    assert!(wildcard.matches("api.example.com"));
    assert!(
        !wildcard.matches("a.b.example.com"),
        "a wildcard covers exactly one label"
    );
    assert!(
        !wildcard.matches("example.com"),
        "a wildcard does not cover the bare domain"
    );
    assert!(
        wildcard.matches("API.Example.COM"),
        "hostnames are case-insensitive"
    );
    assert!(
        wildcard.matches("api.example.com:8443"),
        "the port is decided by the socket, not the route"
    );
}

#[test]
fn an_unknown_port_selects_nothing() {
    let router = router(PRECEDENCE_CONFIG);
    assert!(router.select(9999, &get("/api")).is_none());
    assert_eq!(router.ports().collect::<Vec<_>>(), vec![8080]);
}

#[test]
fn prefix_matches_are_reported_for_rewriting() {
    let router = router(
        r#"
binds:
  - port: 8080
    listeners:
      - routes:
          - name: api
            matches:
              - path:
                  pathPrefix: /api
            backends: [{host: "a:80"}]
"#,
    );

    let selection = router
        .select(8080, &get("/api/v1/tools"))
        .expect("should match");
    assert_eq!(
        selection.matched_prefix.as_deref(),
        Some("/api"),
        "a prefix rewrite needs to know what it is replacing"
    );
}

#[test]
fn a_bad_method_in_config_fails_to_build() {
    let config = Config::from_yaml(
        r#"
binds:
  - port: 8080
    listeners:
      - routes:
          - matches:
              - method: "NOT A METHOD"
            backends: [{host: "a:80"}]
"#,
    )
    .expect("should parse");

    let err = Router::build(&config).expect_err("should not build");
    assert!(err.to_string().contains("method"), "got: {err}");
}

fn cors(yaml: &str) -> CorsMatcher {
    let config = Config::from_yaml(yaml).expect("should parse");
    let policy = config.binds[0].listeners[0].routes[0]
        .policies
        .as_ref()
        .and_then(|p| p.cors.clone())
        .expect("cors policy");
    CorsMatcher::new(&policy)
}

const MCP_CORS: &str = r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - policies:
              cors:
                allowOrigins: ["*"]
                allowHeaders: [mcp-protocol-version, content-type, cache-control]
                exposeHeaders: ["Mcp-Session-Id"]
            backends: [{host: "a:80"}]
"#;

#[test]
fn a_request_without_an_origin_is_not_cors() {
    assert_eq!(cors(MCP_CORS).evaluate(&get("/mcp")), CorsDecision::NotCors);
}

#[test]
fn mcp_session_id_is_exposed_to_browsers() {
    let request = Request::builder()
        .uri("/mcp")
        .header(header::ORIGIN, "https://app.example.com")
        .body(())
        .expect("request should build");

    let CorsDecision::Simple(headers) = cors(MCP_CORS).evaluate(&request) else {
        panic!("a non-preflight cross-origin request should be Simple");
    };
    assert_eq!(
        headers
            .get(header::ACCESS_CONTROL_EXPOSE_HEADERS)
            .and_then(|v| v.to_str().ok()),
        Some("Mcp-Session-Id"),
        "without this a browser client cannot read the session id, and so \
         cannot make a second MCP request"
    );
}

#[test]
fn a_preflight_is_answered_without_calling_the_backend() {
    let request = Request::builder()
        .method(Method::OPTIONS)
        .uri("/mcp")
        .header(header::ORIGIN, "https://app.example.com")
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .body(())
        .expect("request should build");

    let CorsDecision::Preflight(headers) = cors(MCP_CORS).evaluate(&request) else {
        panic!("an OPTIONS request with a requested method is a preflight");
    };
    assert_eq!(
        headers
            .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
            .and_then(|v| v.to_str().ok()),
        Some("mcp-protocol-version, content-type, cache-control")
    );
    assert_eq!(
        headers
            .get(header::ACCESS_CONTROL_ALLOW_METHODS)
            .and_then(|v| v.to_str().ok()),
        Some("POST"),
        "with no allowMethods configured, the requested method is echoed"
    );
}

#[test]
fn a_disallowed_origin_gets_no_allow_header() {
    let matcher = cors(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - policies:
              cors:
                allowOrigins: ["https://trusted.example.com"]
            backends: [{host: "a:80"}]
"#,
    );

    let request = Request::builder()
        .uri("/mcp")
        .header(header::ORIGIN, "https://evil.example.com")
        .body(())
        .expect("request should build");

    let CorsDecision::Simple(headers) = matcher.evaluate(&request) else {
        panic!("expected Simple");
    };
    assert!(
        headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none(),
        "the browser blocks the read; non-browser clients are unaffected"
    );
}

#[test]
fn credentials_force_the_origin_to_be_echoed() {
    let matcher = cors(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - policies:
              cors:
                allowOrigins: ["*"]
                allowCredentials: true
            backends: [{host: "a:80"}]
"#,
    );

    let request = Request::builder()
        .uri("/mcp")
        .header(header::ORIGIN, "https://app.example.com")
        .body(())
        .expect("request should build");

    let CorsDecision::Simple(headers) = matcher.evaluate(&request) else {
        panic!("expected Simple");
    };
    assert_eq!(
        headers
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|v| v.to_str().ok()),
        Some("https://app.example.com"),
        "`*` is invalid alongside credentials, so the origin must be echoed"
    );
    assert_eq!(
        headers.get(header::VARY).and_then(|v| v.to_str().ok()),
        Some("Origin"),
        "an origin-dependent response must not be cached across origins"
    );
}

//! Parser tests, anchored on configuration taken verbatim from agentgateway's
//! own documentation. If upstream YAML stops parsing here, drop-in
//! compatibility has regressed.

use super::*;
use std::time::Duration;

/// Straight from the agentgateway MCP quickstart.
const UPSTREAM_MCP_EXAMPLE: &str = r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - policies:
              cors:
                allowOrigins: ["*"]
                allowHeaders: [mcp-protocol-version, content-type, cache-control]
                exposeHeaders: ["Mcp-Session-Id"]
            backends:
              - mcp:
                  targets:
                    - name: everything
                      stdio:
                        cmd: npx
                        args: ["@modelcontextprotocol/server-everything"]
"#;

#[test]
fn parses_the_upstream_mcp_quickstart() {
    let config = Config::from_yaml(UPSTREAM_MCP_EXAMPLE).expect("upstream example should parse");
    config.validate().expect("upstream example should validate");

    let bind = &config.binds[0];
    assert_eq!(bind.port, 3000);

    let route = &bind.listeners[0].routes[0];
    let cors = route
        .policies
        .as_ref()
        .and_then(|p| p.cors.as_ref())
        .expect("cors policy");
    assert_eq!(cors.allow_origins, ["*"]);
    assert_eq!(cors.expose_headers, ["Mcp-Session-Id"]);

    let BackendTarget::Mcp(mcp) = &route.backends[0].target else {
        panic!("expected an mcp backend, got {:?}", route.backends[0].target);
    };
    let target = &mcp.targets[0];
    assert_eq!(target.name, "everything");
    let McpTargetKind::Stdio(stdio) = &target.kind else {
        panic!("expected a stdio target, got {:?}", target.kind);
    };
    assert_eq!(stdio.cmd, "npx");
    assert_eq!(stdio.args, ["@modelcontextprotocol/server-everything"]);
}

#[test]
fn backend_weight_defaults_to_one() {
    let config = Config::from_yaml(
        r#"
binds:
  - port: 8080
    listeners:
      - routes:
          - backends:
              - host: "10.0.0.1:80"
              - host: "10.0.0.2:80"
                weight: 9
"#,
    )
    .expect("should parse");

    let backends = &config.binds[0].listeners[0].routes[0].backends;
    assert_eq!(backends[0].weight, 1);
    assert_eq!(backends[1].weight, 9);
    assert_eq!(
        backends[0].target,
        BackendTarget::Host("10.0.0.1:80".into())
    );
}

#[test]
fn parses_every_route_match_shape() {
    let config = Config::from_yaml(
        r#"
binds:
  - port: 8080
    listeners:
      - hostname: api.example.com
        protocol: HTTP
        routes:
          - name: everything
            hostnames: [api.example.com]
            matches:
              - path:
                  pathPrefix: /v1
                method: POST
                headers:
                  - name: x-tenant
                    exact: acme
                  - name: x-trace
                    present: true
                query:
                  - name: debug
                    regex: "^(1|true)$"
            backends:
              - host: "backend:8080"
"#,
    )
    .expect("should parse");
    config.validate().expect("should validate");

    let m = &config.binds[0].listeners[0].routes[0].matches[0];
    assert_eq!(m.path, Some(PathMatch::PathPrefix("/v1".into())));
    assert_eq!(m.method.as_deref(), Some("POST"));
    assert_eq!(m.headers[0].value, HeaderMatchValue::Exact("acme".into()));
    assert_eq!(m.headers[1].value, HeaderMatchValue::Present(true));
    assert_eq!(
        m.query[0].value,
        QueryMatchValue::Regex("^(1|true)$".into())
    );
}

#[test]
fn rejects_a_regex_that_does_not_compile() {
    let config = Config::from_yaml(
        r#"
binds:
  - port: 8080
    listeners:
      - routes:
          - matches:
              - path:
                  regex: "["
            backends:
              - host: "backend:8080"
"#,
    )
    .expect("should parse; a bad regex is a validation failure, not a parse one");

    let err = config.validate().expect_err("should not validate");
    assert!(
        err.to_string().contains("regex"),
        "error should name the regex, got: {err}"
    );
}

#[test]
fn rejects_weighting_an_mcp_backend_against_a_host() {
    let config = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - backends:
              - host: "backend:8080"
                weight: 1
              - mcp:
                  targets:
                    - name: t
                      stdio:
                        cmd: echo
                weight: 1
"#,
    )
    .expect("should parse");

    let err = config.validate().expect_err("should not validate");
    assert!(
        err.to_string().contains("MCP backend"),
        "error should explain the MCP restriction, got: {err}"
    );
}

#[test]
fn parses_remote_and_sse_targets_with_defaulted_paths() {
    let config = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - backends:
              - mcp:
                  targets:
                    - name: remote
                      mcp:
                        host: mcp.example.com
                        port: 443
                    - name: legacy
                      sse:
                        host: old.example.com
"#,
    )
    .expect("should parse");

    let BackendTarget::Mcp(mcp) = &config.binds[0].listeners[0].routes[0].backends[0].target else {
        panic!("expected an mcp backend");
    };

    let McpTargetKind::Mcp(remote) = &mcp.targets[0].kind else {
        panic!("expected a streamable http target");
    };
    assert_eq!(remote.port, 443);
    assert_eq!(remote.path, "/mcp", "streamable HTTP default path");

    let McpTargetKind::Sse(legacy) = &mcp.targets[1].kind else {
        panic!("expected an sse target");
    };
    assert_eq!(legacy.port, 80, "port defaults to plain HTTP");
    assert_eq!(legacy.path, "/sse", "sse default path");
}

#[test]
fn tool_name_prefixing_is_the_default() {
    let config = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - backends:
              - mcp:
                  targets:
                    - name: t
                      stdio:
                        cmd: echo
"#,
    )
    .expect("should parse");

    let BackendTarget::Mcp(mcp) = &config.binds[0].listeners[0].routes[0].backends[0].target else {
        panic!("expected an mcp backend");
    };
    assert_eq!(mcp.name_mode, NameMode::Prefix);
}

#[test]
fn durations_parse_and_round_trip() {
    let cases = [
        ("100ms", Duration::from_millis(100)),
        ("5s", Duration::from_secs(5)),
        ("2m", Duration::from_secs(120)),
        ("1h", Duration::from_secs(3600)),
    ];
    for (text, expected) in cases {
        let parsed: DurationString = text.parse().expect("should parse");
        assert_eq!(parsed.0, expected, "parsing {text}");
        assert_eq!(parsed.to_string(), text, "round-tripping {text}");
    }
}

#[test]
fn a_bare_number_is_not_a_duration() {
    // Upstream durations are always suffixed. Accepting `5` would leave the
    // unit to the reader, and "5 what" is exactly the ambiguity that makes a
    // timeout policy dangerous.
    let err = "5".parse::<DurationString>().expect_err("should not parse");
    assert!(err.contains("suffix"), "error should be actionable: {err}");
}

#[test]
fn parses_timeout_retry_and_rate_limit_policies() {
    let config = Config::from_yaml(
        r#"
binds:
  - port: 8080
    listeners:
      - routes:
          - policies:
              timeout:
                requestTimeout: 30s
                backendRequestTimeout: 10s
              retry:
                attempts: 3
                backoff: 100ms
                codes: [502, 503]
              localRateLimit:
                - maxTokens: 100
                  tokensPerFill: 10
                  fillInterval: 1s
              requestHeaderModifier:
                set:
                  x-gateway: rusty
                remove: [x-internal]
            backends:
              - host: "backend:8080"
"#,
    )
    .expect("should parse");

    let policies = config.binds[0].listeners[0].routes[0]
        .policies
        .as_ref()
        .expect("policies");

    let timeout = policies.timeout.as_ref().expect("timeout");
    assert_eq!(
        timeout.request_timeout.map(Duration::from),
        Some(Duration::from_secs(30))
    );

    let retry = policies.retry.as_ref().expect("retry");
    assert_eq!(retry.attempts, 3);
    assert_eq!(retry.codes, [502, 503]);

    let limit = &policies.local_rate_limit[0];
    assert_eq!(limit.max_tokens, 100);
    assert_eq!(limit.kind, RateLimitKind::Requests);

    let modifier = policies
        .request_header_modifier
        .as_ref()
        .expect("header modifier");
    assert_eq!(modifier.set["x-gateway"], "rusty");
    assert_eq!(modifier.remove, ["x-internal"]);
}

#[test]
fn unknown_fields_are_tolerated_not_fatal() {
    // Upstream ships fields faster than we implement them. Refusing to boot on
    // one we do not know would defeat the point of being a drop-in.
    let config = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - someFutureField: {enabled: true}
            backends:
              - host: "backend:8080"
"#,
    )
    .expect("an unknown field should not be fatal");
    config.validate().expect("should validate");
}

#[test]
fn lint_reports_policies_that_parse_but_do_nothing() {
    let config = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - policies:
              extAuthz:
                target: authz:9000
            backends:
              - ai:
                  provider:
                    openAI:
                      model: gpt-4o
"#,
    )
    .expect("should parse");

    let findings = config.lint();
    assert!(
        findings.iter().any(|f| f.contains("extAuthz")),
        "lint should flag extAuthz: {findings:?}"
    );
    assert!(
        !findings.iter().any(|f| f.contains("`ai` backend")),
        "the ai backend is served now and must not be reported as inert: {findings:?}"
    );
}

#[test]
fn a_clean_config_lints_clean() {
    let config = Config::from_yaml(UPSTREAM_MCP_EXAMPLE).expect("should parse");
    assert_eq!(
        config.lint(),
        Vec::<String>::new(),
        "the quickstart config is fully supported and should produce no findings"
    );
}

#[test]
fn config_round_trips_through_yaml() {
    let config = Config::from_yaml(UPSTREAM_MCP_EXAMPLE).expect("should parse");
    let reserialized = serde_yaml::to_string(&config).expect("should serialize");
    let reparsed = Config::from_yaml(&reserialized).expect("should re-parse");
    assert_eq!(config, reparsed);
}

#[test]
fn an_empty_config_is_rejected() {
    let err = Config::from_yaml("binds: []")
        .expect("should parse")
        .validate()
        .expect_err("a gateway that listens nowhere is a mistake, not a config");
    assert!(err.to_string().contains("no binds"));
}

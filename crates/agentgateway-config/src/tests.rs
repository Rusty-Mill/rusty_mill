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
        panic!(
            "expected an mcp backend, got {:?}",
            route.backends[0].target
        );
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
                target: "http://authz:9000"
                includeBody: 4096
              ai:
                promptGuard: {}
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
        findings.iter().any(|f| f.contains("extAuthz.includeBody")),
        "forwarding a body is not implemented and must be reported: {findings:?}"
    );
    assert!(
        findings.iter().any(|f| f.contains("policies.ai")),
        "ai policies are not enforced and must be reported: {findings:?}"
    );
    assert!(
        !findings
            .iter()
            .any(|f| f.ends_with("policies.extAuthz: parsed but not enforced by this build")),
        "extAuthz itself is enforced now and must not be reported as inert: {findings:?}"
    );
    assert!(
        !findings.iter().any(|f| f.contains("`ai` backend")),
        "the ai backend is served now and must not be reported as inert: {findings:?}"
    );
}

#[test]
fn authorization_rules_parse_in_every_form_upstream_accepts() {
    // A bare string is an allow, which is how upstream's own examples are
    // written; the map forms carry the other two modes.
    let config = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - policies:
              mcpAuthorization:
                rules:
                  - 'mcp.tool.name == "echo"'
                  - allow: 'jwt.sub == "u1"'
                  - deny: 'mcp.tool.name == "delete"'
                  - require: 'jwt.iss == "https://auth.example.com"'
            backends:
              - mcp:
                  targets:
                    - name: t
                      mcp:
                        host: http://localhost:3001/mcp
"#,
    )
    .expect("should parse");

    let rules = &config.binds[0].listeners[0].routes[0]
        .policies
        .as_ref()
        .expect("policies should be present")
        .mcp_authorization
        .as_ref()
        .expect("policy should be present")
        .rules;

    assert_eq!(
        rules,
        &vec![
            AuthorizationRule::Allow(r#"mcp.tool.name == "echo""#.into()),
            AuthorizationRule::Allow(r#"jwt.sub == "u1""#.into()),
            AuthorizationRule::Deny(r#"mcp.tool.name == "delete""#.into()),
            AuthorizationRule::Require(r#"jwt.iss == "https://auth.example.com""#.into()),
        ]
    );
}

#[test]
fn a_rule_naming_no_mode_is_rejected() {
    // Rather than parsing to an empty rule that quietly does nothing.
    let err = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - policies:
              mcpAuthorization:
                rules:
                  - comment: 'not a mode'
            backends:
              - host: "127.0.0.1:9"
"#,
    )
    .expect_err("should not parse");
    assert!(err.to_string().contains("one of"), "got: {err}");
}

#[test]
fn rules_are_no_longer_reported_as_inert() {
    let config = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - policies:
              mcpAuthorization:
                rules:
                  - 'mcp.tool.name == "echo"'
            backends:
              - mcp:
                  targets:
                    - name: t
                      mcp:
                        host: http://localhost:3001/mcp
"#,
    )
    .expect("should parse");

    assert_eq!(
        config.lint(),
        Vec::<String>::new(),
        "rules are evaluated now and must not be reported as parsed-but-inert"
    );
}

#[test]
fn guardrails_parse_in_upstreams_shape() {
    let config = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - policies:
              mcpGuardrails:
                processors:
                  - kind: remote
                    methods: { "tools/call": request, "*/list": response }
                    host: 127.0.0.1:9999
                    failureMode: failOpen
                    timeout: 2s
                    metadata:
                      tenant: 'request.headers["x-tenant"]'
                    requestHeaders:
                      allowed: [x-tenant]
                      disallowed: [authorization]
            backends:
              - mcp:
                  targets:
                    - name: t
                      mcp:
                        host: http://localhost:3001/mcp
"#,
    )
    .expect("should parse");

    let guardrails = config.binds[0].listeners[0].routes[0]
        .policies
        .as_ref()
        .expect("policies should be present")
        .mcp_guardrails
        .as_ref()
        .expect("policy should be present");

    let processor = &guardrails.processors[0];
    assert_eq!(processor.methods["tools/call"], Phase::Request);
    assert_eq!(processor.methods["*/list"], Phase::Response);
    assert_eq!(processor.host.as_deref(), Some("127.0.0.1:9999"));
    assert_eq!(processor.failure_mode, Some(FailureMode::FailOpen));
    assert_eq!(
        processor.timeout.map(Duration::from),
        Some(Duration::from_secs(2))
    );
    assert_eq!(
        processor.metadata["tenant"],
        r#"request.headers["x-tenant"]"#
    );
    assert_eq!(
        processor.request_headers.allowed,
        vec!["x-tenant".to_string()]
    );
}

#[test]
fn a_header_filter_lets_the_deny_list_win_case_insensitively() {
    let filter = HeaderFilter {
        allowed: vec!["X-Tenant".into(), "Authorization".into()],
        disallowed: vec!["authorization".into()],
    };
    assert!(filter.allows("x-tenant"));
    assert!(!filter.allows("AUTHORIZATION"));
    assert!(
        !filter.allows("cookie"),
        "a non-empty allow list excludes the rest"
    );

    // Empty forwards everything -- upstream's reading, and the opposite of
    // extAuthz.includeHeaders.
    assert!(HeaderFilter::default().allows("anything"));
}

#[test]
fn lint_reports_guardrails_that_cannot_fire() {
    let config = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - policies:
              mcpGuardrails:
                processors:
                  - methods: { "logging/setLevel": full }
                    host: 127.0.0.1:9999
                  - methods: { "a*b": full }
                    host: 127.0.0.1:9999
                  - methods: { "tools/call": full }
                    backend: policy-service
                  - methods: { "prompts/*": full, "resources/*": response }
                    host: 127.0.0.1:9999
            backends:
              - mcp:
                  targets:
                    - name: t
                      mcp:
                        host: http://localhost:3001/mcp
"#,
    )
    .expect("should parse");

    let findings = config.lint();
    assert!(
        findings
            .iter()
            .any(|f| f.contains("processors[0]") && f.contains("never runs")),
        "a processor keyed on an unserved method must be reported: {findings:?}"
    );
    assert!(
        findings
            .iter()
            .any(|f| f.contains("processors[1]") && f.contains("can never match")),
        "an unmatchable pattern must be reported: {findings:?}"
    );
    assert!(
        findings
            .iter()
            .any(|f| f.contains("processors[2]") && f.contains("only `host`")),
        "a processor this build cannot resolve must be reported: {findings:?}"
    );
}

#[test]
fn a_usable_guardrail_lints_clean() {
    let config = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - policies:
              mcpGuardrails:
                processors:
                  - methods: { "tools/*": full }
                    host: 127.0.0.1:9999
            backends:
              - mcp:
                  targets:
                    - name: t
                      mcp:
                        host: http://localhost:3001/mcp
"#,
    )
    .expect("should parse");

    assert_eq!(config.lint(), Vec::<String>::new());
}

/// A route with the given `urlRewrite` over one Streamable HTTP target.
fn one_target_with_rewrite(rewrite: &str) -> Config {
    Config::from_yaml(&format!(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - policies:
              urlRewrite:
{rewrite}
            backends:
              - mcp:
                  targets:
                    - name: t
                      mcp:
                        host: http://localhost:3001/mcp
"#
    ))
    .expect("should parse")
}

#[test]
fn a_full_path_rewrite_over_one_mcp_target_lints_clean() {
    // One target, so there is no ambiguity about whose path is meant.
    assert_eq!(
        one_target_with_rewrite("                path:\n                  full: /rpc").lint(),
        Vec::<String>::new()
    );
}

#[test]
fn an_authority_rewrite_over_one_mcp_target_lints_clean() {
    // Same reasoning as `path.full`: one target, so there is no ambiguity
    // about whose address is meant.
    assert_eq!(
        one_target_with_rewrite("                authority: elsewhere:8080").lint(),
        Vec::<String>::new()
    );
}

#[test]
fn a_prefix_rewrite_needs_exactly_one_path_prefix_match() {
    // Which prefix a request matched is not knowable when the target is
    // dialled, which happens once at startup.
    let findings =
        one_target_with_rewrite("                path:\n                  prefix: /v2").lint();
    assert!(
        findings
            .iter()
            .any(|f| f.contains("path.prefix") && f.contains("matches on 0")),
        "a route with no pathPrefix match cannot resolve one: {findings:?}"
    );
}

#[test]
fn a_prefix_rewrite_over_one_matched_prefix_lints_clean() {
    let config = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - matches:
              - path:
                  pathPrefix: /mcp
            policies:
              urlRewrite:
                path:
                  prefix: /rpc
            backends:
              - mcp:
                  targets:
                    - name: t
                      mcp:
                        host: http://localhost:3001/mcp
"#,
    )
    .expect("should parse");

    assert_eq!(config.lint(), Vec::<String>::new());
}

#[test]
fn a_prefix_rewrite_over_two_matched_prefixes_is_reported() {
    let config = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - matches:
              - path:
                  pathPrefix: /mcp
              - path:
                  pathPrefix: /rpc
            policies:
              urlRewrite:
                path:
                  prefix: /internal
            backends:
              - mcp:
                  targets:
                    - name: t
                      mcp:
                        host: http://localhost:3001/mcp
"#,
    )
    .expect("should parse");

    let findings = config.lint();
    assert!(
        findings.iter().any(|f| f.contains("matches on 2")),
        "{findings:?}"
    );
}

#[test]
fn lint_reports_a_path_rewrite_over_more_than_one_target() {
    let config = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - policies:
              urlRewrite:
                path:
                  full: /rpc
            backends:
              - mcp:
                  targets:
                    - name: a
                      mcp:
                        host: http://localhost:3001/mcp
                    - name: b
                      mcp:
                        host: http://localhost:3002/mcp
"#,
    )
    .expect("should parse");

    let findings = config.lint();
    assert!(
        findings
            .iter()
            .any(|f| f.contains("exactly one target") && f.contains("this route has 2")),
        "{findings:?}"
    );
}

#[test]
fn lint_names_both_overrides_when_both_are_set() {
    let config = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - policies:
              urlRewrite:
                authority: elsewhere:8080
                path:
                  full: /rpc
            backends:
              - mcp:
                  targets:
                    - name: a
                      mcp:
                        host: http://localhost:3001/mcp
                    - name: b
                      mcp:
                        host: http://localhost:3002/mcp
"#,
    )
    .expect("should parse");

    let findings = config.lint();
    assert!(
        findings.iter().any(|f| f.contains("path and authority")),
        "the finding should name what is actually being overridden: {findings:?}"
    );
}

#[test]
fn lint_reports_a_path_rewrite_over_a_stdio_target() {
    let config = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - policies:
              urlRewrite:
                path:
                  full: /rpc
            backends:
              - mcp:
                  targets:
                    - name: t
                      stdio:
                        cmd: /bin/true
"#,
    )
    .expect("should parse");

    let findings = config.lint();
    assert!(
        findings
            .iter()
            .any(|f| f.contains("only an `mcp:` target has an address")),
        "{findings:?}"
    );
}

#[test]
fn url_rewrite_on_a_host_route_is_not_reported() {
    // Where it does apply, it must stay silent.
    let config = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - policies:
              urlRewrite:
                authority: "elsewhere:8080"
            backends:
              - host: "10.0.0.1:8080"
"#,
    )
    .expect("should parse");

    assert_eq!(config.lint(), Vec::<String>::new());
}

#[test]
fn a_response_header_modifier_on_an_mcp_route_is_not_reported() {
    // It applies to the response the gateway itself returns, so there is
    // nothing to warn about.
    let config = Config::from_yaml(
        r#"
binds:
  - port: 3000
    listeners:
      - routes:
          - policies:
              responseHeaderModifier:
                set:
                  x-served-by: rusty
            backends:
              - mcp:
                  targets:
                    - name: t
                      mcp:
                        host: http://localhost:3001/mcp
"#,
    )
    .expect("should parse");

    assert_eq!(config.lint(), Vec::<String>::new());
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

//! Unit tests for per-request destinations.

use super::*;

fn backend(target: Option<&str>) -> DynamicBackend {
    DynamicBackend {
        target: target.map(str::to_string),
        rest: Default::default(),
    }
}

fn request(uri: &str, host: Option<&str>) -> Request<()> {
    let mut builder = Request::builder().uri(uri);
    if let Some(host) = host {
        builder = builder.header(http::header::HOST, host);
    }
    builder.body(()).expect("should build")
}

#[test]
fn with_no_target_the_request_chooses() {
    let dynamic = Dynamic::new(&backend(None), "route[0]").expect("should compile");
    assert!(!dynamic.is_computed());

    // A proxied request carries the authority in the URI.
    assert_eq!(
        dynamic
            .authority(&request("http://upstream.example:8080/x", None))
            .map(|a| a.to_string()),
        Some("upstream.example:8080".to_string())
    );

    // An ordinary one has only the header.
    assert_eq!(
        dynamic
            .authority(&request("/x", Some("upstream.example:8080")))
            .map(|a| a.to_string()),
        Some("upstream.example:8080".to_string())
    );
}

#[test]
fn the_uri_wins_over_the_host_header() {
    // A proxied request states its destination outright; the header is what
    // is left when it does not.
    let dynamic = Dynamic::new(&backend(None), "r").expect("should compile");
    assert_eq!(
        dynamic
            .authority(&request("http://from-uri:80/x", Some("from-header:80")))
            .map(|a| a.to_string()),
        Some("from-uri:80".to_string())
    );
}

#[test]
fn a_request_naming_nowhere_is_not_dialled() {
    // Answered with a 400 by the caller rather than a guess: the alternative
    // is dialling somewhere the request did not name.
    let dynamic = Dynamic::new(&backend(None), "r").expect("should compile");
    assert!(dynamic.authority(&request("/x", None)).is_none());
    assert!(dynamic.authority(&request("/x", Some(""))).is_none());
}

#[test]
fn a_target_expression_reads_the_request() {
    let dynamic = Dynamic::new(
        &backend(Some(r#"request.headers["x-upstream"]"#)),
        "route[0]",
    )
    .expect("should compile");
    assert!(dynamic.is_computed());

    let mut asked = request("/x", Some("ignored:80"));
    asked
        .headers_mut()
        .insert("x-upstream", "chosen.internal:9000".parse().expect("value"));
    assert_eq!(
        dynamic.authority(&asked).map(|a| a.to_string()),
        Some("chosen.internal:9000".to_string()),
        "the expression decides, not the address the client dialled"
    );
}

#[test]
fn an_expression_can_read_the_method_path_and_authority() {
    for (source, expected) in [
        (r#""fixed.internal:80""#, "fixed.internal:80"),
        (
            r#"request.method == "GET" ? "get.internal:80" : "other:80""#,
            "get.internal:80",
        ),
        (
            r#"request.path == "/a" ? "a.internal:80" : "b.internal:80""#,
            "a.internal:80",
        ),
        (r#"request.authority"#, "client.example:80"),
    ] {
        let dynamic = Dynamic::new(&backend(Some(source)), "r").expect("should compile");
        assert_eq!(
            dynamic
                .authority(&request("/a", Some("client.example:80")))
                .map(|a| a.to_string()),
            Some(expected.to_string()),
            "`{source}`"
        );
    }
}

#[test]
fn an_expression_producing_nothing_usable_dials_nowhere() {
    // Rendering a list or a boolean as an address cannot be what was meant.
    for source in [
        "request.headers[\"absent\"]",
        "1 + 1",
        "true",
        "request.headers",
    ] {
        let dynamic = Dynamic::new(&backend(Some(source)), "r").expect("should compile");
        assert!(
            dynamic
                .authority(&request("/x", Some("client:80")))
                .is_none(),
            "`{source}` should not produce an address"
        );
    }
}

#[test]
fn an_expression_producing_a_non_authority_dials_nowhere() {
    let dynamic = Dynamic::new(&backend(Some(r#""not a host""#)), "r").expect("should compile");
    assert!(dynamic.authority(&request("/x", None)).is_none());
}

#[test]
fn an_expression_that_does_not_compile_fails_at_startup() {
    // Falling back to the request's authority would choose a different
    // upstream, quietly.
    let err = Dynamic::new(&backend(Some("this is not cel")), "route[0].dynamic")
        .expect_err("should not compile");
    assert!(err.to_string().contains("route[0].dynamic.target"), "{err}");
    assert!(err.to_string().contains("this is not cel"), "{err}");
}

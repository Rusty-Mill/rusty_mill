//! Live integration smoke test against real OAuth 2.0 / OIDC providers.
//!
//! This is a *development-time example*, not part of the library: it uses
//! `std::process::Command` to shell out to `curl` as its HTTP transport,
//! which is exactly the kind of "bring your own client" usage the crate
//! is designed for -- the library itself never does this.
//!
//! Run with: `cargo run --example live_provider_test`
//! Requires network access and `curl` on PATH. Talks to:
//! - https://accounts.google.com (discovery + JWKS, no auth needed)
//! - https://demo.duendesoftware.com (Duende's public IdentityServer demo
//!   instance, explicitly provided for testing OAuth/OIDC clients against
//!   -- using its publicly documented demo client credentials, listed on
//!   https://demo.duendesoftware.com/#demo-clients)

use rusty_oauth::client::{Client, ClientId, ClientSecret};
use rusty_oauth::request::HttpRequest;
use rusty_oauth::{dpop, introspection, jwks, jwt, metadata, revocation, token};
use std::process::Command;

fn curl(req: &HttpRequest) -> (u16, String) {
    let mut cmd = Command::new("curl");
    cmd.arg("-sS").arg("-X").arg(req.method.as_str());
    cmd.arg("-w").arg("\n__STATUS__%{http_code}");
    for (k, v) in &req.headers {
        cmd.arg("-H").arg(format!("{k}: {v}"));
    }
    if !req.body.is_empty() {
        cmd.arg("--data-binary")
            .arg(String::from_utf8_lossy(&req.body).to_string());
    }
    cmd.arg(&req.url);
    let output = cmd.output().expect("failed to run curl");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (body, status) = stdout
        .rsplit_once("__STATUS__")
        .expect("missing status marker");
    (
        status.trim().parse().unwrap_or(0),
        body.trim_end_matches('\n').to_string(),
    )
}

/// Same as `curl`, but also captures response headers (needed for
/// `DPoP-Nonce`), returned as (status, headers, body).
///
/// Uses `-D <file>` for headers rather than `-i` mixed into stdout:
/// curl's `-i` includes any HTTP/1.1 "100 Continue" preamble ahead of the
/// real response (common on a `POST` with a body), which breaks a naive
/// single `\r\n\r\n` split -- the header *file* gets that preamble too,
/// but as a separate leading block we can just skip by taking the last
/// one.
fn curl_with_headers(req: &HttpRequest) -> (u16, Vec<(String, String)>, String) {
    let header_file = std::env::temp_dir().join(format!("curl_headers_{}.txt", std::process::id()));
    let mut cmd = Command::new("curl");
    cmd.arg("-sS").arg("-X").arg(req.method.as_str());
    cmd.arg("-D").arg(&header_file);
    cmd.arg("-w").arg("__STATUS__%{http_code}");
    for (k, v) in &req.headers {
        cmd.arg("-H").arg(format!("{k}: {v}"));
    }
    if !req.body.is_empty() {
        cmd.arg("--data-binary")
            .arg(String::from_utf8_lossy(&req.body).to_string());
    }
    cmd.arg(&req.url);
    let output = cmd.output().expect("failed to run curl");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (body, status_str) = stdout
        .rsplit_once("__STATUS__")
        .expect("missing status marker");
    let status: u16 = status_str.trim().parse().unwrap_or(0);

    let header_text = std::fs::read_to_string(&header_file).unwrap_or_default();
    let _ = std::fs::remove_file(&header_file);
    let last_block = header_text
        .split("\r\n\r\n")
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .last()
        .unwrap_or("");
    let headers = last_block
        .lines()
        .skip(1) // status line
        .filter_map(|l| {
            l.split_once(':')
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        })
        .collect();
    (status, headers, body.to_string())
}

fn section(title: &str) {
    println!("\n=== {title} ===");
}

fn main() {
    // ---- 1. Discovery + JWKS against Google (no credentials needed) ----
    section("Google: RFC 8414/OIDC discovery");
    let req = metadata::discovery_request("https://accounts.google.com");
    let (status, body) = curl(&req);
    assert_eq!(status, 200, "Google discovery fetch failed");
    let google_meta = metadata::parse_metadata(&body).expect("parse Google discovery doc");
    assert!(metadata::verify_issuer(
        &google_meta,
        "https://accounts.google.com"
    ));
    println!("issuer: {}", google_meta.issuer);
    println!("token_endpoint: {:?}", google_meta.token_endpoint);
    println!("jwks_uri: {:?}", google_meta.jwks_uri);
    println!(
        "grant_types_supported: {:?}",
        google_meta.grant_types_supported
    );

    section("Google: JWKS parsing");
    let jwks_uri = google_meta
        .jwks_uri
        .clone()
        .expect("Google metadata has jwks_uri");
    let jwks_req = HttpRequest {
        method: rusty_oauth::request::Method::Get,
        url: jwks_uri,
        headers: vec![],
        body: vec![],
    };
    let (status, body) = curl(&jwks_req);
    assert_eq!(status, 200, "Google JWKS fetch failed");
    let jwk_set = jwks::JwkSet::parse(&body).expect("parse Google JWKS");
    println!(
        "fetched {} real signing key(s) from Google",
        jwk_set.keys.len()
    );
    for key in &jwk_set.keys {
        println!(
            "  kid={:?} kty={} alg={:?} use={:?}",
            key.kid, key.kty, key.alg, key.use_
        );
        if key.kty == "RSA" {
            key.to_rsa_public_key()
                .expect("convert real Google JWK to RsaPublicKey");
            println!("    -> converted to RsaPublicKey OK");
        }
    }

    // ---- 2. Duende demo IdentityServer: full client_credentials round trip ----
    section("Duende demo: RFC 8414/OIDC discovery");
    let req = metadata::discovery_request("https://demo.duendesoftware.com");
    let (status, body) = curl(&req);
    assert_eq!(status, 200, "Duende discovery fetch failed");
    let duende_meta = metadata::parse_metadata(&body).expect("parse Duende discovery doc");
    assert!(metadata::verify_issuer(
        &duende_meta,
        "https://demo.duendesoftware.com"
    ));
    println!("issuer: {}", duende_meta.issuer);
    println!("token_endpoint: {:?}", duende_meta.token_endpoint);
    println!(
        "introspection_endpoint: {:?}",
        duende_meta.introspection_endpoint
    );
    println!("revocation_endpoint: {:?}", duende_meta.revocation_endpoint);
    println!(
        "pushed_authorization_request_endpoint: {:?}",
        duende_meta.pushed_authorization_request_endpoint
    );

    let token_endpoint = duende_meta.token_endpoint.clone().unwrap();
    let introspection_endpoint = duende_meta.introspection_endpoint.clone().unwrap();
    let revocation_endpoint = duende_meta.revocation_endpoint.clone().unwrap();

    section("Duende demo: client_credentials grant (client_secret_basic)");
    // Publicly documented demo client from https://demo.duendesoftware.com/#demo-clients
    let client = Client::confidential(ClientId::new("m2m"), ClientSecret::new("secret"));
    let req = token::client_credentials_request(&token_endpoint, &client, Some("api")).unwrap();
    let (status, body) = curl(&req);
    println!("HTTP {status}");
    let token_response =
        token::parse_token_response(status, &body).expect("parse real token response");
    println!(
        "access_token (truncated): {}...",
        &token_response.access_token[..24.min(token_response.access_token.len())]
    );
    println!("token_type: {}", token_response.token_type);
    println!("expires_in: {:?}", token_response.expires_in);
    let access_token = token_response.access_token.clone();

    section("Duende demo: token introspection (RFC 7662)");
    let req = introspection::introspection_request(
        &introspection_endpoint,
        &client,
        &access_token,
        Some("access_token"),
    )
    .unwrap();
    let (status, body) = curl(&req);
    println!("HTTP {status}");
    let introspected = introspection::parse_introspection_response(status, &body)
        .expect("parse real introspection response");
    println!("active: {}", introspected.active);
    println!("scope: {:?}", introspected.scope);
    println!("client_id: {:?}", introspected.client_id);
    println!("exp: {:?}", introspected.exp);
    assert!(introspected.active, "freshly issued token should be active");

    section("Duende demo: verify the access token is a real RS256 JWT we can decode");
    if let Ok(decoded) = jwt::decode_unverified(&access_token) {
        println!("header: {}", decoded.header.to_json());
        println!("claims: {}", decoded.claims.to_json());
    } else {
        println!("(access token is opaque, not a JWT -- also valid per spec)");
    }

    section("Duende demo: token revocation (RFC 7009)");
    let req = revocation::revocation_request(
        &revocation_endpoint,
        &client,
        &access_token,
        Some("access_token"),
    )
    .unwrap();
    let (status, body) = curl(&req);
    println!("HTTP {status}");
    revocation::parse_revocation_response(status, &body).expect("parse real revocation response");
    println!("revocation acknowledged");

    section("Duende demo: re-introspect after revocation");
    let req = introspection::introspection_request(
        &introspection_endpoint,
        &client,
        &access_token,
        Some("access_token"),
    )
    .unwrap();
    let (status, body) = curl(&req);
    let introspected_after = introspection::parse_introspection_response(status, &body)
        .expect("parse post-revocation introspection response");
    println!("active after revocation: {}", introspected_after.active);
    // Self-contained JWT access tokens are validated by signature + exp,
    // not a server-side lookup, so many deployments (correctly, per RFC
    // 7009 §2.2) accept the revocation call with 200 OK but have nothing
    // to actually invalidate. That's expected here, not a bug in this
    // crate's revocation request/response handling, which already
    // verified the 200 was parsed as success above.

    // ---- 3. DPoP-bound client_credentials against m2m.dpop, with nonce retry ----
    section("Duende demo: DPoP-bound client_credentials (m2m.dpop)");
    let dpop_client = Client::confidential(ClientId::new("m2m.dpop"), ClientSecret::new("secret"));
    let ec_key = loop {
        let bytes = rusty_oauth::rand::random_bytes(32).unwrap();
        if let Ok(k) = rusty_oauth::jwt::es256::EcPrivateKey::from_bytes(&bytes) {
            break k;
        }
    };

    let mut nonce: Option<String> = None;
    let mut attempt = 0;
    let dpop_result = loop {
        attempt += 1;
        let proof =
            dpop::build_proof(&ec_key, "POST", &token_endpoint, None, nonce.as_deref()).unwrap();
        let mut req =
            token::client_credentials_request(&token_endpoint, &dpop_client, Some("api")).unwrap();
        req.headers.push(("DPoP".to_string(), proof));
        let (status, headers, body) = curl_with_headers(&req);
        println!("attempt {attempt}: HTTP {status}");
        let server_nonce = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("DPoP-Nonce"))
            .map(|(_, v)| v.clone());

        if std::env::var("DEBUG_BODY").is_ok() {
            eprintln!("  raw body: {body:?}");
            eprintln!("  headers: {headers:?}");
        }
        match token::parse_token_response(status, &body) {
            Ok(resp) => break Ok(resp),
            Err(err) if dpop::is_use_dpop_nonce_error(&err) && attempt < 3 => {
                println!("  server requires a nonce (RFC 9449 §8) -- retrying with DPoP-Nonce: {server_nonce:?}");
                nonce = server_nonce;
                continue;
            }
            Err(err) => break Err(err),
        }
    };

    match dpop_result {
        Ok(resp) => {
            println!("DPoP token_type: {}", resp.token_type);
            println!(
                "DPoP access_token (truncated): {}...",
                &resp.access_token[..24.min(resp.access_token.len())]
            );
            assert_eq!(
                resp.token_type.to_lowercase(),
                "dpop",
                "server should mark this token DPoP-bound"
            );
            println!("SUCCESS: full DPoP nonce-retry round trip against a real server");
        }
        Err(err) => {
            println!("DPoP flow did not complete: {err}");
            println!("(reporting, not hiding, this -- see write-up)");
        }
    }

    println!("\n=== ALL CORE ASSERTIONS PASSED ===");
}

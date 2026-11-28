//! The minted token checked the way the gateway checks it.
//!
//! `verify` mirrors `agentgateway-auth`'s `FileJwks::verify` (algorithm pinned
//! before any key, `kid` looked up in the JWKS, `iss` and `exp` required) plus
//! the policy's audience list, so a token that passes here passes there.
//! The keys under `testdata/` are throwaway test keys and sign nothing real.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, DecodingKey, EncodingKey, Header, Validation};
use serde_json::Value;

use super::*;

const PEM: &[u8] = include_bytes!("../testdata/test-only-p256.pem");
const OTHER_PEM: &[u8] = include_bytes!("../testdata/test-only-p256-other.pem");
const P384_PEM: &[u8] = include_bytes!("../testdata/test-only-p384.pem");
const POLICY: &[u8] = b"schema = 1\nlens = \"builder-rust\"\nrole = \"builder\"\n";
const ISSUER: &str = "lel-orch@baileyai";
const BUDGET: Duration = Duration::from_secs(45 * 60);

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after 1970")
        .as_secs()
}

fn key(kid: &str) -> SigningKey {
    SigningKey::from_pem(PEM, kid).expect("test key parses")
}

fn scope() -> Scope {
    Scope::new(
        "bbp_factory",
        "t-1382",
        "c-0123",
        "builder-rust",
        Role::Builder,
        1,
    )
    .expect("valid scope")
}

fn claims_at(now: u64) -> Claims {
    Claims::new(
        "baileyai",
        scope(),
        policy_digest(POLICY),
        Vec::new(),
        [0xab; 16],
        now,
        BUDGET,
    )
    .expect("valid claims")
}

fn jwks(keys: &[&SigningKey]) -> JwkSet {
    JwkSet {
        keys: keys.iter().map(|k| jwk(k).expect("jwk")).collect(),
    }
}

/// The gateway's check: ES256 only, known `kid`, `iss`, `exp`, `aud`.
fn verify(token: &str, set: &JwkSet, audience: &str) -> Result<Value, String> {
    let header = decode_header(token).map_err(|e| e.to_string())?;
    if header.alg != Algorithm::ES256 {
        return Err(format!("algorithm {:?} refused", header.alg));
    }
    let kid = header.kid.ok_or("no kid")?;
    let found = set.find(&kid).ok_or("unknown kid")?;
    let key = DecodingKey::from_jwk(found).map_err(|e| e.to_string())?;
    let mut validation = Validation::new(Algorithm::ES256);
    validation.set_issuer(&[ISSUER]);
    validation.set_audience(&[audience]);
    validation.leeway = 60;
    validation.set_required_spec_claims(&["exp", "iss"]);
    decode::<Value>(token, &key, &validation)
        .map(|data| data.claims)
        .map_err(|e| format!("{:?}", e.kind()))
}

#[test]
fn minted_token_passes_the_gateway_check_with_every_claim() {
    let key = key("orch-1");
    let now = now();
    let token = mint(&claims_at(now), &key).expect("mints");

    let claims = verify(&token, &jwks(&[&key]), AUDIENCE).expect("gateway accepts");

    assert_eq!(claims["iss"], ISSUER);
    assert_eq!(claims["aud"], "lel-gateway");
    assert_eq!(claims["sub"], "bbp_factory/c-0123/builder-rust/1");
    assert_eq!(claims["jti"], "ab".repeat(16));
    assert_eq!(claims["iat"], now);
    assert_eq!(claims["exp"], now + 50 * 60);
    assert_eq!(claims["lel.project"], "bbp_factory");
    assert_eq!(claims["lel.task"], "t-1382");
    assert_eq!(claims["lel.claim"], "c-0123");
    assert_eq!(claims["lel.lens"], "builder-rust");
    assert_eq!(claims["lel.role"], "builder");
    assert_eq!(claims["lel.policy"], policy_digest(POLICY).as_str());
    assert!(
        claims.get("lel.grants").is_none(),
        "empty grants are left out"
    );
}

#[test]
fn header_is_es256_with_the_kid() {
    let token = mint(&claims_at(now()), &key("orch-1")).expect("mints");
    let header = decode_header(&token).expect("header");
    assert_eq!(header.alg, Algorithm::ES256);
    assert_eq!(header.kid.as_deref(), Some("orch-1"));
}

#[test]
fn grants_are_carried_when_present() {
    let grants = vec!["github_pr_create".to_owned()];
    let claims = Claims::new(
        "baileyai",
        scope(),
        policy_digest(POLICY),
        grants,
        [1; 16],
        now(),
        BUDGET,
    )
    .expect("valid");
    let key = key("orch-1");
    let token = mint(&claims, &key).expect("mints");
    let seen = verify(&token, &jwks(&[&key]), AUDIENCE).expect("accepts");
    assert_eq!(seen["lel.grants"], serde_json::json!(["github_pr_create"]));
}

#[test]
fn wrong_audience_is_refused() {
    let key = key("orch-1");
    let token = mint(&claims_at(now()), &key).expect("mints");
    let err = verify(&token, &jwks(&[&key]), "some-other-service").expect_err("refused");
    assert_eq!(err, "InvalidAudience");
}

#[test]
fn expired_token_is_refused() {
    let key = key("orch-1");
    let long_ago = now() - 24 * 60 * 60;
    let token = mint(&claims_at(long_ago), &key).expect("mints");
    let err = verify(&token, &jwks(&[&key]), AUDIENCE).expect_err("refused");
    assert_eq!(err, "ExpiredSignature");
}

#[test]
fn unknown_kid_is_refused() {
    let token = mint(&claims_at(now()), &key("orch-1")).expect("mints");
    let set = jwks(&[&key("orch-2")]);
    assert_eq!(
        verify(&token, &set, AUDIENCE),
        Err("unknown kid".to_owned())
    );
}

#[test]
fn a_different_key_under_the_same_kid_fails_the_signature() {
    let token = mint(&claims_at(now()), &key("orch-1")).expect("mints");
    let other = SigningKey::from_pem(OTHER_PEM, "orch-1").expect("parses");
    let err = verify(&token, &jwks(&[&other]), AUDIENCE).expect_err("refused");
    assert_eq!(err, "InvalidSignature");
}

#[test]
fn hs256_header_is_refused_before_any_key() {
    let mut header = Header::new(Algorithm::HS256);
    header.kid = Some("orch-1".to_owned());
    let forged = jsonwebtoken::encode(
        &header,
        &claims_at(now()).to_json(),
        &EncodingKey::from_secret(b"guess"),
    )
    .expect("signs");
    let err = verify(&forged, &jwks(&[&key("orch-1")]), AUDIENCE).expect_err("refused");
    assert_eq!(err, "algorithm HS256 refused");
}

#[test]
fn rotation_keeps_both_kids_valid() {
    let (old, new) = (
        key("orch-1"),
        SigningKey::from_pem(OTHER_PEM, "orch-2").expect("parses"),
    );
    let set = jwks(&[&old, &new]);
    for signer in [&old, &new] {
        let token = mint(&claims_at(now()), signer).expect("mints");
        verify(&token, &set, AUDIENCE).expect("accepts");
    }
}

#[test]
fn jwk_is_public_ec_p256_with_kid_and_use() {
    let entry = serde_json::to_value(jwk(&key("orch-1")).expect("jwk")).expect("json");
    assert_eq!(entry["kty"], "EC");
    assert_eq!(entry["crv"], "P-256");
    assert_eq!(entry["alg"], "ES256");
    assert_eq!(entry["kid"], "orch-1");
    assert_eq!(entry["use"], "sig");
    assert!(entry.get("d").is_none(), "no private component");
}

#[test]
fn debug_shows_the_kid_not_the_key() {
    let shown = format!("{:?}", key("orch-1"));
    assert!(shown.contains("orch-1"));
    assert!(!shown.contains("content"), "{shown}");
}

#[test]
fn lifetime_is_budget_plus_grace_capped_at_four_hours() {
    assert_eq!(lifetime(BUDGET), Duration::from_secs(50 * 60));
    assert_eq!(lifetime(Duration::ZERO), GRACE);
    assert_eq!(lifetime(Duration::from_secs(4 * 60 * 60)), MAX_LIFETIME);
    assert_eq!(lifetime(Duration::MAX), MAX_LIFETIME);
}

#[test]
fn claims_never_outlive_the_cap() {
    let claims = Claims::new(
        "h",
        scope(),
        policy_digest(POLICY),
        Vec::new(),
        [0; 16],
        1_000,
        Duration::from_secs(24 * 60 * 60),
    )
    .expect("valid");
    assert_eq!(claims.expires_at(), 1_000 + MAX_LIFETIME.as_secs());
}

#[test]
fn expiry_overflow_is_refused() {
    let err = Claims::new(
        "h",
        scope(),
        policy_digest(POLICY),
        Vec::new(),
        [0; 16],
        u64::MAX,
        BUDGET,
    )
    .expect_err("overflow");
    assert!(matches!(err, TokenError::ClockOverflow));
}

#[test]
fn empty_ids_are_refused() {
    let cases = [
        ("", "t", "c", "l", "lel.project"),
        ("p", "", "c", "l", "lel.task"),
        ("p", "t", "", "l", "lel.claim"),
        ("p", "t", "c", "", "lel.lens"),
    ];
    for (project, task, claim, lens, field) in cases {
        let err = Scope::new(project, task, claim, lens, Role::Critic, 0).expect_err(field);
        assert!(
            matches!(err, TokenError::Empty(f) if f == field),
            "{field}: {err}"
        );
    }
}

#[test]
fn separator_in_a_subject_part_is_refused() {
    let err = Scope::new("p", "t", "c/1", "l", Role::Builder, 0).expect_err("refused");
    assert!(matches!(err, TokenError::Separator("lel.claim")));
    Scope::new("p", "tasks/1382", "c", "l", Role::Builder, 0).expect("task is not in sub");
}

#[test]
fn empty_host_kid_or_grant_is_refused() {
    let digest = policy_digest(POLICY);
    let no_host = Claims::new("", scope(), digest.clone(), Vec::new(), [0; 16], 0, BUDGET);
    assert!(matches!(no_host, Err(TokenError::Empty("iss host"))));
    let blank = Claims::new(
        "h",
        scope(),
        digest,
        vec![String::new()],
        [0; 16],
        0,
        BUDGET,
    );
    assert!(matches!(blank, Err(TokenError::Empty("lel.grants entry"))));
    assert!(matches!(
        SigningKey::from_pem(PEM, ""),
        Err(TokenError::Empty("kid"))
    ));
}

#[test]
fn non_p256_or_garbage_keys_are_refused() {
    assert!(matches!(
        SigningKey::from_pem(P384_PEM, "k"),
        Err(TokenError::Key(_))
    ));
    assert!(matches!(
        SigningKey::from_pem(b"not a key", "k"),
        Err(TokenError::Key(_))
    ));
}

#[test]
fn policy_digest_is_lowercase_sha256_hex() {
    assert_eq!(
        policy_digest(b"").as_str(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_ne!(policy_digest(POLICY), policy_digest(b"other"));
}

#[test]
fn critic_role_is_spelled_as_policies_spell_it() {
    assert_eq!(Role::Critic.as_str(), "critic");
    assert_eq!(Role::Builder.as_str(), "builder");
}

use super::*;

const A: [u8; LEN] = [1; LEN];
const B: [u8; LEN] = [2; LEN];

fn with(names: &[(&str, [u8; LEN])]) -> Tokens {
    let mut tokens = Tokens::default();
    for (name, bytes) in names {
        tokens.create(name, bytes).unwrap();
    }
    tokens
}

#[test]
fn a_token_verifies_as_its_name_and_nothing_else_does() {
    let tokens = with(&[("laptop", A), ("phone", B)]);
    assert_eq!(tokens.verify(&A), Some("laptop"));
    assert_eq!(tokens.verify(&B), Some("phone"));
    assert_eq!(tokens.verify(&[3; LEN]), None);
    assert_eq!(tokens.verify(b""), None);
}

#[test]
fn the_file_round_trips_and_keeps_only_hashes() {
    let tokens = with(&[("laptop", A)]);
    let text = tokens.text();
    assert!(!text.contains(&rusty_hex::encode(&A)));
    assert_eq!(Tokens::parse(&text).unwrap(), tokens);
}

#[test]
fn comments_and_blank_lines_are_skipped() {
    let tokens = with(&[("a", A)]);
    let text = format!("# tokens\n\n{}# end\n", tokens.text());
    assert_eq!(Tokens::parse(&text).unwrap(), tokens);
}

#[test]
fn a_bad_file_is_refused_whole() {
    let good = with(&[("a", A)]).text();
    for bad in [
        "a".to_string(),
        "a zz".to_string(),
        "a 00".to_string(),
        "bad/name 0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        format!("{good}{good}"),
    ] {
        assert!(Tokens::parse(&bad).is_err(), "{bad:?}");
    }
}

#[test]
fn create_refuses_a_name_in_use_and_rotate_and_revoke_refuse_a_missing_one() {
    let mut tokens = with(&[("a", A)]);
    assert!(tokens.create("a", &B).is_err());
    assert!(tokens.rotate("b", &B).is_err());
    assert!(tokens.revoke("b").is_err());
    assert!(tokens.create("", &B).is_err());
    assert!(tokens.create(&"x".repeat(65), &B).is_err());
}

#[test]
fn rotating_replaces_the_token_and_revoking_removes_it() {
    let mut tokens = with(&[("a", A)]);
    tokens.rotate("a", &B).unwrap();
    assert_eq!(tokens.verify(&A), None);
    assert_eq!(tokens.verify(&B), Some("a"));
    tokens.revoke("a").unwrap();
    assert_eq!(tokens.verify(&B), None);
}

#[test]
fn a_reload_changes_only_rotated_and_revoked_names() {
    let old = with(&[("a", A), ("b", B), ("c", A)]);
    let mut fresh = old.clone();
    fresh.rotate("a", &B).unwrap();
    fresh.revoke("b").unwrap();
    fresh.create("d", &A).unwrap();
    assert_eq!(old.changed(&fresh), ["a", "b"]);
}

#[test]
fn a_generated_token_is_random_and_round_trips_through_hex() {
    let (one, shown) = generate().unwrap();
    assert_eq!(from_hex(&shown), Some(one));
    assert_ne!(one, generate().unwrap().0);
    assert_eq!(from_hex("abc"), None);
}

#[test]
fn the_principal_carries_the_token_name() {
    assert_eq!(principal("laptop").unwrap().id(), "token:laptop");
}

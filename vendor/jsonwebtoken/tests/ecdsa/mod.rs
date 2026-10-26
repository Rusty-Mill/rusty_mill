use serde::{Deserialize, Serialize};
#[cfg(feature = "use_pem")]
use time::OffsetDateTime;
use wasm_bindgen_test::wasm_bindgen_test;

use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey,
    crypto::{sign, verify},
};
#[cfg(feature = "use_pem")]
use jsonwebtoken::{Header, Validation, decode, encode};

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Claims {
    sub: String,
    company: String,
    exp: i64,
}

#[test]
#[wasm_bindgen_test]
fn round_trip_sign_verification_pk8() {
    let privkey = include_bytes!("private_ecdsa_key.pk8");
    let pubkey = include_bytes!("public_ecdsa_key.pk8");

    let encrypted =
        sign(b"hello world", &EncodingKey::from_ec_der(privkey), Algorithm::ES256).unwrap();
    let is_valid =
        verify(&encrypted, b"hello world", &DecodingKey::from_ec_der(pubkey), Algorithm::ES256)
            .unwrap();
    assert!(is_valid);
}

#[cfg(feature = "use_pem")]
#[test]
#[wasm_bindgen_test]
fn round_trip_sign_verification_pem() {
    let privkey_pem = include_bytes!("private_ecdsa_key.pem");
    let pubkey_pem = include_bytes!("public_ecdsa_key.pem");
    let encrypted =
        sign(b"hello world", &EncodingKey::from_ec_pem(privkey_pem).unwrap(), Algorithm::ES256)
            .unwrap();
    let is_valid = verify(
        &encrypted,
        b"hello world",
        &DecodingKey::from_ec_pem(pubkey_pem).unwrap(),
        Algorithm::ES256,
    )
    .unwrap();
    assert!(is_valid);
}

#[cfg(feature = "use_pem")]
#[test]
#[wasm_bindgen_test]
fn round_trip_claim() {
    let privkey_pem = include_bytes!("private_ecdsa_key.pem");
    let pubkey_pem = include_bytes!("public_ecdsa_key.pem");
    let my_claims = Claims {
        sub: "b@b.com".to_string(),
        company: "ACME".to_string(),
        exp: OffsetDateTime::now_utc().unix_timestamp() + 10000,
    };
    let token = encode(
        &Header::new(Algorithm::ES256),
        &my_claims,
        &EncodingKey::from_ec_pem(privkey_pem).unwrap(),
    )
    .unwrap();
    let token_data = decode::<Claims>(
        &token,
        &DecodingKey::from_ec_pem(pubkey_pem).unwrap(),
        &Validation::new(Algorithm::ES256),
    )
    .unwrap();
    assert_eq!(my_claims, token_data.claims);
}

#[cfg(feature = "use_pem")]
#[test]
#[wasm_bindgen_test]
fn ec_x_y() {
    let privkey = include_str!("private_ecdsa_key.pem");
    let my_claims = Claims {
        sub: "b@b.com".to_string(),
        company: "ACME".to_string(),
        exp: OffsetDateTime::now_utc().unix_timestamp() + 10000,
    };
    let x = "w7JAoU_gJbZJvV-zCOvU9yFJq0FNC_edCMRM78P8eQQ";
    let y = "wQg1EytcsEmGrM70Gb53oluoDbVhCZ3Uq3hHMslHVb4";

    let encrypted = encode(
        &Header::new(Algorithm::ES256),
        &my_claims,
        &EncodingKey::from_ec_pem(privkey.as_ref()).unwrap(),
    )
    .unwrap();
    let res = decode::<Claims>(
        &encrypted,
        &DecodingKey::from_ec_components(x, y).unwrap(),
        &Validation::new(Algorithm::ES256),
    );
    assert!(res.is_ok());
}

#[cfg(feature = "use_pem")]
#[test]
#[wasm_bindgen_test]
fn ed_jwk() {
    use jsonwebtoken::jwk::Jwk;
    use serde_json::json;

    let privkey = include_str!("private_ecdsa_key.pem");
    let my_claims = Claims {
        sub: "b@b.com".to_string(),
        company: "ACME".to_string(),
        exp: OffsetDateTime::now_utc().unix_timestamp() + 10000,
    };
    let jwk: Jwk = serde_json::from_value(json!({
        "kty": "EC",
        "crv": "P-256",
        "x": "w7JAoU_gJbZJvV-zCOvU9yFJq0FNC_edCMRM78P8eQQ",
        "y": "wQg1EytcsEmGrM70Gb53oluoDbVhCZ3Uq3hHMslHVb4",
        "kid": "ec01",
        "alg": "ES256",
        "use": "sig"
    }))
    .unwrap();

    let encrypted = encode(
        &Header::new(Algorithm::ES256),
        &my_claims,
        &EncodingKey::from_ec_pem(privkey.as_ref()).unwrap(),
    )
    .unwrap();
    let res = decode::<Claims>(
        &encrypted,
        &DecodingKey::from_jwk(&jwk).unwrap(),
        &Validation::new(Algorithm::ES256),
    );
    assert!(res.is_ok());
}

// https://jwt.io/ is often used for examples so ensure their example works with jsonwebtoken
#[cfg(feature = "use_pem")]
#[test]
#[wasm_bindgen_test]
fn roundtrip_with_jwtio_example() {
    // We currently do not support SEC1 so we use the converted PKCS8 formatted
    let privkey_pem = include_bytes!("private_jwtio_pkcs8.pem");
    let pubkey_pem = include_bytes!("public_jwtio.pem");
    let my_claims = Claims {
        sub: "b@b.com".to_string(),
        company: "ACME".to_string(),
        exp: OffsetDateTime::now_utc().unix_timestamp() + 10000,
    };
    let token = encode(
        &Header::new(Algorithm::ES384),
        &my_claims,
        &EncodingKey::from_ec_pem(privkey_pem).unwrap(),
    )
    .unwrap();
    let token_data = decode::<Claims>(
        &token,
        &DecodingKey::from_ec_pem(pubkey_pem).unwrap(),
        &Validation::new(Algorithm::ES384),
    )
    .unwrap();
    assert_eq!(my_claims, token_data.claims);
}

#[cfg(feature = "use_pem")]
#[test]
#[wasm_bindgen_test]
fn ec_jwk_from_key() {
    use jsonwebtoken::jwk::Jwk;
    use serde_json::json;

    let privkey = include_str!("private_ecdsa_key.pem");
    let encoding_key = EncodingKey::from_ec_pem(privkey.as_ref()).unwrap();
    let jwk = Jwk::from_encoding_key(&encoding_key, Algorithm::ES256).unwrap();
    assert_eq!(
        jwk,
        serde_json::from_value(json!({
            "kty": "EC",
            "crv": "P-256",
            "x": "w7JAoU_gJbZJvV-zCOvU9yFJq0FNC_edCMRM78P8eQQ",
            "y": "wQg1EytcsEmGrM70Gb53oluoDbVhCZ3Uq3hHMslHVb4",
            "alg": "ES256",
        }))
        .unwrap()
    );
}

// Regression for the signature 2/3 adapter: malformed signatures and changed
// messages must still fail for both migrated curves, with raw r || s encoding.
#[cfg(feature = "use_pem")]
#[test]
fn migrated_curves_reject_invalid_signatures() {
    for (algorithm, private, public, width) in [
        (
            Algorithm::ES256,
            include_bytes!("private_ecdsa_key.pem").as_slice(),
            include_bytes!("public_ecdsa_key.pem").as_slice(),
            64,
        ),
        (
            Algorithm::ES384,
            include_bytes!("private_jwtio_pkcs8.pem").as_slice(),
            include_bytes!("public_jwtio.pem").as_slice(),
            96,
        ),
    ] {
        let signing = EncodingKey::from_ec_pem(private).unwrap();
        let verifying = DecodingKey::from_ec_pem(public).unwrap();
        let signature = sign(b"migration compatibility", &signing, algorithm).unwrap();
        // Generated with the checksum-verified, unmodified jsonwebtoken 11.0.0
        // archive and its original lockfile (P-256/P-384 0.13). Preserve the
        // exact deterministic signatures as well as verifying legacy tokens.
        let legacy = match algorithm {
            Algorithm::ES256 => "pxYdXQ0eiikWKbhWhSl2E_NFueW0qDjhZm8XC1ICClf6CPfkEN3wERUB8fcnsI0d5UBpF-fLzug27jbQse2vaw",
            Algorithm::ES384 => "A4CpY6JVF0hbo3zcqX4VMeQyqzH4kbJS5n2wYKDuLcF2Sge0c7IDgzJ1LG5NKK4-OQrOV9uThZXEDj-V9LyfOHn92P8rRbBjvJzh2R8m4T1LtgtSrnU0SQyxDyATZuvW",
            _ => unreachable!(),
        };
        assert_eq!(signature, legacy);
        assert!(verify(legacy, b"migration compatibility", &verifying, algorithm).unwrap());
        let raw = base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            &signature,
        )
        .unwrap();
        assert_eq!(raw.len(), width);
        assert!(
            verify(
                &signature,
                b"migration compatibility",
                &verifying,
                algorithm
            )
            .unwrap()
        );
        assert!(!verify(&signature, b"changed message", &verifying, algorithm).unwrap());
        let zeros = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            vec![0; width],
        );
        assert!(!verify(&zeros, b"migration compatibility", &verifying, algorithm).unwrap());
        // The public verify API treats provider errors as a failed signature.
        assert!(!verify("AA", b"migration compatibility", &verifying, algorithm).unwrap());
    }
}

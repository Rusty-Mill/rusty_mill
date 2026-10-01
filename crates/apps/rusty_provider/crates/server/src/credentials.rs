//! Resolving the caller credentials the config names, at startup (design
//! review 3.1). A credential the operator configured but the environment
//! does not provide is a startup error, never a warning followed by a server
//! that authenticates less than it was told to (or, with nothing left, not at
//! all).

use std::collections::HashMap;
use std::sync::Arc;

use rp_router::Config;

use crate::jwt::JwtVerifier;

/// The caller credentials a config resolves to.
pub struct Credentials {
    /// `server.api_key_env`'s value, when configured.
    pub api_key: Option<String>,
    /// Each `[[clients]]` key, mapped to the client's name and rate limit.
    pub client_keys: HashMap<String, (String, u32)>,
    /// `[jwt]`'s verifier, when configured.
    pub jwt: Option<Arc<JwtVerifier>>,
}

/// Resolve every credential `config` names through `env` (the process
/// environment in `main`, a map in tests).
///
/// # Errors
/// One line per configured credential that did not resolve: an unset or
/// empty `server.api_key_env`, an unset or empty `[[clients]].api_key_env`,
/// or a `[jwt]` section with neither a usable `hs256_secret_env` nor a
/// `jwks_url`.
pub fn resolve_credentials(
    config: &Config,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Credentials, String> {
    let lookup = |var: &str| env(var).filter(|v| !v.is_empty());
    let mut missing = Vec::new();

    let api_key = config.server.api_key_env.as_ref().and_then(|var| {
        let key = lookup(var);
        if key.is_none() {
            missing.push(format!("server.api_key_env: {var} is not set"));
        }
        key
    });

    let mut client_keys = HashMap::new();
    for client in &config.clients {
        match lookup(&client.api_key_env) {
            Some(key) => {
                client_keys.insert(key, (client.name.clone(), client.requests_per_minute));
            }
            None => missing.push(format!(
                "[[clients]] {}: {} is not set",
                client.name, client.api_key_env
            )),
        }
    }

    let jwt = match &config.jwt {
        None => None,
        Some(cfg) => {
            let secret = cfg.hs256_secret_env.as_deref().and_then(lookup);
            match JwtVerifier::new(cfg, secret) {
                Some(verifier) => Some(Arc::new(verifier)),
                None => {
                    missing.push(
                        "[jwt]: neither hs256_secret_env resolves nor jwks_url is set".into(),
                    );
                    None
                }
            }
        }
    };

    if !missing.is_empty() {
        return Err(format!(
            "configured credentials did not resolve; refusing to start with less \
             authentication than configured:\n  {}",
            missing.join("\n  ")
        ));
    }
    Ok(Credentials {
        api_key,
        client_keys,
        jwt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |var| map.get(var).cloned()
    }

    fn config(toml: &str) -> Config {
        Config::from_toml_str(toml).unwrap()
    }

    const FULL: &str = r#"
        providers = {}
        [server]
        api_key_env = "RP_KEY"
        [[clients]]
        name = "ci"
        api_key_env = "CI_KEY"
        requests_per_minute = 60
        [jwt]
        hs256_secret_env = "JWT_SECRET"
    "#;

    #[test]
    fn every_configured_credential_resolves() {
        let got = resolve_credentials(
            &config(FULL),
            &env(&[("RP_KEY", "k"), ("CI_KEY", "c"), ("JWT_SECRET", "s")]),
        )
        .unwrap();
        assert_eq!(got.api_key.as_deref(), Some("k"));
        assert_eq!(got.client_keys["c"].0, "ci");
        assert!(got.jwt.is_some());
    }

    /// Design review 3.1: each configured-but-unresolved credential is named
    /// and startup is refused (it used to warn and serve with less auth).
    #[test]
    fn an_unresolved_configured_credential_refuses_startup() {
        let err = resolve_credentials(&config(FULL), &env(&[("CI_KEY", "c"), ("RP_KEY", "")]))
            .err()
            .expect("refused");
        assert!(err.contains("server.api_key_env: RP_KEY"), "{err}");
        assert!(err.contains("[jwt]"), "{err}");
        assert!(!err.contains("CI_KEY"), "{err}");

        let err = resolve_credentials(&config(FULL), &env(&[("RP_KEY", "k"), ("JWT_SECRET", "s")]))
            .err()
            .expect("refused");
        assert!(err.contains("[[clients]] ci: CI_KEY"), "{err}");
    }

    #[test]
    fn no_configured_credentials_is_not_an_error() {
        let got = resolve_credentials(&config("providers = {}"), &env(&[])).unwrap();
        assert!(got.api_key.is_none() && got.client_keys.is_empty() && got.jwt.is_none());
    }
}

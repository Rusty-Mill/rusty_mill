//! Operator-facing config/ops tooling for rusty_provider -- config
//! validation, resolved-provider listing, and API-key-env presence
//! checks. Not a rewrite of the config schema: every report here is built
//! from `rp_router::Config`, the exact same type (and TOML parsing) the
//! real server loads at startup, so there is nothing for this crate to
//! drift out of sync with.
//!
//! Deliberately dependency-light -- no argument-parsing crate, no async
//! runtime. This is a synchronous, read-only inspection tool: it never
//! makes a network call and never needs one.

use rp_router::{Config, ProviderKind};

pub mod setup;

/// One `[providers.*]` entry's resolution status, mirroring exactly what
/// `Router::from_config` itself decides at startup (an unresolvable
/// `api_key_env` is skipped with a warning, not a hard failure) -- so
/// this report never disagrees with what the real server would do with
/// the same config and environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderReport {
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub api_key_env: String,
    pub active: bool,
}

/// Builds one `ProviderReport` per `[providers.*]` entry, sorted by name
/// for stable output. `env_lookup` is injected so tests can check
/// resolution logic without mutating real process environment variables;
/// `rp-cli`'s own `main` passes `std::env::var` here.
pub fn provider_reports(
    config: &Config,
    env_lookup: impl Fn(&str) -> Option<String>,
) -> Vec<ProviderReport> {
    let mut reports: Vec<ProviderReport> = config
        .providers
        .iter()
        .map(|(name, cfg)| ProviderReport {
            name: name.clone(),
            kind: cfg.kind,
            base_url: cfg.base_url.clone(),
            api_key_env: cfg.api_key_env.clone(),
            active: env_lookup(&cfg.api_key_env).is_some_and(|v| !v.is_empty()),
        })
        .collect();
    reports.sort_by(|a, b| a.name.cmp(&b.name));
    reports
}

/// One environment variable this config references (never its value --
/// only whether it's currently set), for `rp-cli keys check`. Covers
/// every `*_env` field across the config: provider/client API keys, the
/// shared server/admin tokens, and the optional auxiliary backends
/// (persistence, webhook, moderation, web search).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEnvReport {
    pub label: String,
    pub env_var: String,
    pub set: bool,
}

/// Builds one `KeyEnvReport` per `*_env` field this config declares,
/// covering every section that reads a secret from the environment.
/// Fields left unset in config (e.g. no `[webhook]` at all) simply don't
/// contribute a row -- there's no env var name to check.
pub fn key_env_reports(
    config: &Config,
    env_lookup: impl Fn(&str) -> Option<String>,
) -> Vec<KeyEnvReport> {
    let mut rows = Vec::new();
    let mut push = |label: String, env_var: &str| {
        let set = env_lookup(env_var).is_some_and(|v| !v.is_empty());
        rows.push(KeyEnvReport {
            label,
            env_var: env_var.to_string(),
            set,
        });
    };

    if let Some(env_var) = &config.server.api_key_env {
        push("server.api_key_env".to_string(), env_var);
    }
    if let Some(env_var) = &config.server.admin_key_env {
        push("server.admin_key_env".to_string(), env_var);
    }
    let mut provider_names: Vec<&String> = config.providers.keys().collect();
    provider_names.sort();
    for name in provider_names {
        let cfg = &config.providers[name];
        push(format!("providers.{name}.api_key_env"), &cfg.api_key_env);
    }
    for client in &config.clients {
        push(
            format!("clients.{}.api_key_env", client.name),
            &client.api_key_env,
        );
    }
    if let Some(persistence) = &config.persistence {
        if let Some(env_var) = &persistence.postgres_url_env {
            push("persistence.postgres_url_env".to_string(), env_var);
        }
    }
    if let Some(webhook) = &config.webhook {
        if let Some(env_var) = &webhook.auth_header_env {
            push("webhook.auth_header_env".to_string(), env_var);
        }
    }
    if let Some(moderation) = &config.moderation {
        push(
            "moderation.api_key_env".to_string(),
            &moderation.api_key_env,
        );
    }
    if let Some(web_search) = &config.web_search {
        push(
            "web_search.api_key_env".to_string(),
            &web_search.api_key_env,
        );
    }
    if let Some(jwt) = &config.jwt {
        if let Some(env_var) = &jwt.hs256_secret_env {
            push("jwt.hs256_secret_env".to_string(), env_var);
        }
    }

    rows
}

/// One `[[guardrails]]` entry's regex-compile result, for `rp-cli config
/// check`. `Router::from_config` already does exactly this check at
/// startup (skipping an invalid pattern with a warning rather than
/// refusing to start) -- this surfaces the same check up front, before a
/// deploy, rather than only in a runtime log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardrailCheck {
    pub name: String,
    pub error: Option<String>,
}

pub fn check_guardrails(config: &Config) -> Vec<GuardrailCheck> {
    config
        .guardrails
        .iter()
        .map(|g| GuardrailCheck {
            name: g.name.clone(),
            error: regex::Regex::new(&g.pattern).err().map(|e| e.to_string()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_providers(entries: &[(&str, &str)]) -> Config {
        let mut toml = String::new();
        for (name, api_key_env) in entries {
            toml.push_str(&format!(
                "[providers.{name}]\nkind = \"openai\"\nbase_url = \"https://example.com\"\napi_key_env = \"{api_key_env}\"\n\n"
            ));
        }
        Config::from_toml_str(&toml).expect("valid test config")
    }

    #[test]
    fn provider_reports_marks_a_resolvable_key_as_active() {
        let config = config_with_providers(&[("openai", "OPENAI_KEY")]);
        let reports = provider_reports(&config, |var| {
            (var == "OPENAI_KEY").then(|| "sk-test".to_string())
        });
        assert_eq!(reports.len(), 1);
        assert!(reports[0].active);
    }

    #[test]
    fn provider_reports_marks_an_unset_env_var_as_inactive() {
        let config = config_with_providers(&[("openai", "OPENAI_KEY")]);
        let reports = provider_reports(&config, |_| None);
        assert_eq!(reports.len(), 1);
        assert!(!reports[0].active);
    }

    #[test]
    fn provider_reports_treats_an_empty_value_the_same_as_unset() {
        // Matches Router::from_config's own `Ok(k) if !k.is_empty()` rule.
        let config = config_with_providers(&[("openai", "OPENAI_KEY")]);
        let reports = provider_reports(&config, |_| Some(String::new()));
        assert!(!reports[0].active);
    }

    #[test]
    fn provider_reports_is_sorted_by_name() {
        let config = config_with_providers(&[("zeta", "Z_KEY"), ("alpha", "A_KEY")]);
        let reports = provider_reports(&config, |_| None);
        assert_eq!(reports[0].name, "alpha");
        assert_eq!(reports[1].name, "zeta");
    }

    #[test]
    fn key_env_reports_includes_every_configured_provider_and_client() {
        let toml = r#"
            [providers.openai]
            kind = "openai"
            base_url = "https://example.com"
            api_key_env = "OPENAI_KEY"

            [[clients]]
            name = "acme"
            api_key_env = "ACME_KEY"
            requests_per_minute = 60
        "#;
        let config = Config::from_toml_str(toml).unwrap();
        let rows = key_env_reports(&config, |_| None);
        let env_vars: Vec<&str> = rows.iter().map(|r| r.env_var.as_str()).collect();
        assert!(env_vars.contains(&"OPENAI_KEY"));
        assert!(env_vars.contains(&"ACME_KEY"));
    }

    #[test]
    fn key_env_reports_reports_set_state_per_variable() {
        let toml = r#"
            [providers.openai]
            kind = "openai"
            base_url = "https://example.com"
            api_key_env = "OPENAI_KEY"
        "#;
        let config = Config::from_toml_str(toml).unwrap();
        let rows = key_env_reports(&config, |var| {
            (var == "OPENAI_KEY").then(|| "sk-test".to_string())
        });
        assert_eq!(rows.len(), 1);
        assert!(rows[0].set);
    }

    #[test]
    fn key_env_reports_omits_sections_that_are_not_configured() {
        let config = Config::from_toml_str("providers = {}").unwrap();
        let rows = key_env_reports(&config, |_| None);
        assert!(rows.is_empty());
    }

    #[test]
    fn key_env_reports_includes_jwt_hs256_secret_env_when_configured() {
        let toml = r#"
            providers = {}

            [jwt]
            hs256_secret_env = "JWT_SECRET"
        "#;
        let config = Config::from_toml_str(toml).unwrap();
        let rows = key_env_reports(&config, |var| {
            (var == "JWT_SECRET").then(|| "s3cret".to_string())
        });
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "jwt.hs256_secret_env");
        assert!(rows[0].set);
    }

    #[test]
    fn key_env_reports_omits_jwt_row_when_only_jwks_url_is_set() {
        // jwks_url has no *_env field of its own (it's a plain URL, not a
        // secret) -- only hs256_secret_env belongs in this report.
        let toml = r#"
            providers = {}

            [jwt]
            jwks_url = "https://example.com/jwks.json"
        "#;
        let config = Config::from_toml_str(toml).unwrap();
        let rows = key_env_reports(&config, |_| None);
        assert!(rows.is_empty());
    }

    #[test]
    fn check_guardrails_reports_no_error_for_a_valid_pattern() {
        let toml = r#"
            providers = {}

            [[guardrails]]
            name = "no-ssn"
            pattern = '\d{3}-\d{2}-\d{4}'
            action = "block"
        "#;
        let config = Config::from_toml_str(toml).unwrap();
        let checks = check_guardrails(&config);
        assert_eq!(checks.len(), 1);
        assert!(checks[0].error.is_none());
    }

    #[test]
    fn check_guardrails_reports_an_error_for_an_invalid_pattern() {
        let toml = r#"
            providers = {}

            [[guardrails]]
            name = "broken"
            pattern = '('
            action = "block"
        "#;
        let config = Config::from_toml_str(toml).unwrap();
        let checks = check_guardrails(&config);
        assert_eq!(checks.len(), 1);
        assert!(checks[0].error.is_some());
    }
}

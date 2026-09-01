//! Shared configuration for the `rusty_meshed` crate family -- the Rust
//! port of meshed's `meshed.core.config.PlatformConfig` (env-prefixed
//! `MESHED_*` settings: Kafka bootstrap servers, Schema Registry URL,
//! registry DB path/base URL, default topic partitions/replication/
//! retention).
//!
//! See `../../capability-manifest.md` (rows XFM-041..049, SDK-081..086)
//! for the source capabilities this crate covers.

use rusty_err::Error;

/// Environment-variable prefix every [`PlatformConfig`] field is read
/// under, matching `meshed.core.config.PlatformConfig`'s
/// `SettingsConfigDict(env_prefix="MESHED_")` (XFM-048).
pub const ENV_PREFIX: &str = "MESHED_";

/// Typed configuration for the meshed platform, ported from
/// `meshed.core.config.PlatformConfig`. Every field is overridable via an
/// environment variable prefixed [`ENV_PREFIX`] (e.g.
/// `MESHED_KAFKA_BOOTSTRAP_SERVERS=broker:9092`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformConfig {
    /// Bootstrap server list for Kafka clients (XFM-041).
    pub kafka_bootstrap_servers: String,
    /// Schema Registry base URL; no trailing slash by convention, not
    /// enforced by validation (XFM-042).
    pub schema_registry_url: String,
    /// Default partition count for new topics (XFM-043).
    pub default_num_partitions: u32,
    /// Default replication factor for new topics (XFM-044).
    pub default_replication_factor: u32,
    /// Default retention period, in milliseconds, for event/command
    /// topics. `2_592_000_000` ms = 30 days (XFM-045).
    pub default_retention_ms: u64,
    /// SQLite database path for the data product registry, relative to
    /// the process working directory (XFM-046).
    pub registry_db_path: String,
    /// Data Product Registry REST API base URL (XFM-047).
    pub registry_base_url: String,
}

impl Default for PlatformConfig {
    fn default() -> Self {
        PlatformConfig {
            kafka_bootstrap_servers: "localhost:9092".to_string(),
            schema_registry_url: "http://localhost:8081".to_string(),
            default_num_partitions: 3,
            default_replication_factor: 1,
            default_retention_ms: 2_592_000_000,
            registry_db_path: "meshed_registry.db".to_string(),
            registry_base_url: "http://localhost:8000".to_string(),
        }
    }
}

/// Errors raised while building a [`PlatformConfig`] -- the Rust
/// equivalent of pydantic's field validation on `PlatformConfig` (`ge=1`
/// on the three topic-default fields).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    /// `{0}` names the offending env var; `{1}` is the raw value that
    /// failed to parse as an integer.
    #[error("{0} is not a valid integer: {1}")]
    InvalidInt(&'static str, String),
    /// `{0}` names the offending env var; `{1}` is the parsed value that
    /// violated its `>= 1` constraint.
    #[error("{0} must be >= 1, got {1}")]
    OutOfRange(&'static str, i64),
}

impl PlatformConfig {
    /// Builds a [`PlatformConfig`] from the process environment, matching
    /// `PlatformConfig()`'s behavior in the Python source: reads fresh on
    /// every call, so a later env var change is picked up without a
    /// process restart (REG-008).
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_source(|key| std::env::var(key).ok())
    }

    /// Builds a [`PlatformConfig`] from an arbitrary key lookup rather
    /// than the real process environment -- the seam [`from_env`] is
    /// built on, and what this crate's own tests use so they never touch
    /// (or race on) actual process env vars.
    ///
    /// [`from_env`]: PlatformConfig::from_env
    pub fn from_source(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let defaults = PlatformConfig::default();

        let kafka_bootstrap_servers =
            env_var(&get, "KAFKA_BOOTSTRAP_SERVERS").unwrap_or(defaults.kafka_bootstrap_servers);
        let schema_registry_url =
            env_var(&get, "SCHEMA_REGISTRY_URL").unwrap_or(defaults.schema_registry_url);
        let registry_db_path =
            env_var(&get, "REGISTRY_DB_PATH").unwrap_or(defaults.registry_db_path);
        let registry_base_url =
            env_var(&get, "REGISTRY_BASE_URL").unwrap_or(defaults.registry_base_url);

        let default_num_partitions = match env_var(&get, "DEFAULT_NUM_PARTITIONS") {
            Some(raw) => parse_positive_u32(&raw, "MESHED_DEFAULT_NUM_PARTITIONS")?,
            None => defaults.default_num_partitions,
        };
        let default_replication_factor = match env_var(&get, "DEFAULT_REPLICATION_FACTOR") {
            Some(raw) => parse_positive_u32(&raw, "MESHED_DEFAULT_REPLICATION_FACTOR")?,
            None => defaults.default_replication_factor,
        };
        let default_retention_ms = match env_var(&get, "DEFAULT_RETENTION_MS") {
            Some(raw) => parse_positive_u64(&raw, "MESHED_DEFAULT_RETENTION_MS")?,
            None => defaults.default_retention_ms,
        };

        Ok(PlatformConfig {
            kafka_bootstrap_servers,
            schema_registry_url,
            default_num_partitions,
            default_replication_factor,
            default_retention_ms,
            registry_db_path,
            registry_base_url,
        })
    }
}

fn env_var(get: &impl Fn(&str) -> Option<String>, suffix: &str) -> Option<String> {
    get(&format!("{ENV_PREFIX}{suffix}"))
}

fn parse_positive_u32(raw: &str, field: &'static str) -> Result<u32, ConfigError> {
    let value: i64 = raw
        .parse()
        .map_err(|_| ConfigError::InvalidInt(field, raw.to_string()))?;
    if value < 1 {
        return Err(ConfigError::OutOfRange(field, value));
    }
    u32::try_from(value).map_err(|_| ConfigError::InvalidInt(field, raw.to_string()))
}

fn parse_positive_u64(raw: &str, field: &'static str) -> Result<u64, ConfigError> {
    let value: i64 = raw
        .parse()
        .map_err(|_| ConfigError::InvalidInt(field, raw.to_string()))?;
    if value < 1 {
        return Err(ConfigError::OutOfRange(field, value));
    }
    u64::try_from(value).map_err(|_| ConfigError::InvalidInt(field, raw.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn source(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    #[test]
    fn defaults_when_no_env_vars_set() {
        let config = PlatformConfig::from_source(source(&[])).unwrap();
        assert_eq!(config, PlatformConfig::default());
        assert_eq!(config.kafka_bootstrap_servers, "localhost:9092");
        assert_eq!(config.schema_registry_url, "http://localhost:8081");
        assert_eq!(config.default_num_partitions, 3);
        assert_eq!(config.default_replication_factor, 1);
        assert_eq!(config.default_retention_ms, 2_592_000_000);
        assert_eq!(config.registry_db_path, "meshed_registry.db");
        assert_eq!(config.registry_base_url, "http://localhost:8000");
    }

    #[test]
    fn every_field_is_overridable_via_its_prefixed_env_var() {
        let config = PlatformConfig::from_source(source(&[
            ("MESHED_KAFKA_BOOTSTRAP_SERVERS", "broker:9092"),
            ("MESHED_SCHEMA_REGISTRY_URL", "http://schema:8081"),
            ("MESHED_DEFAULT_NUM_PARTITIONS", "6"),
            ("MESHED_DEFAULT_REPLICATION_FACTOR", "3"),
            ("MESHED_DEFAULT_RETENTION_MS", "86400000"),
            ("MESHED_REGISTRY_DB_PATH", "/tmp/registry.db"),
            ("MESHED_REGISTRY_BASE_URL", "http://registry:8000"),
        ]))
        .unwrap();

        assert_eq!(config.kafka_bootstrap_servers, "broker:9092");
        assert_eq!(config.schema_registry_url, "http://schema:8081");
        assert_eq!(config.default_num_partitions, 6);
        assert_eq!(config.default_replication_factor, 3);
        assert_eq!(config.default_retention_ms, 86_400_000);
        assert_eq!(config.registry_db_path, "/tmp/registry.db");
        assert_eq!(config.registry_base_url, "http://registry:8000");
    }

    #[test]
    fn unprefixed_env_var_is_ignored() {
        // Matches PlatformConfig's env_prefix="MESHED_": a bare
        // KAFKA_BOOTSTRAP_SERVERS (no MESHED_ prefix) must not override
        // the setting. This is the same distinction capability-manifest.md's
        // DOM group flags: run_continuous.py/run_scenario.py read the
        // unprefixed variable directly, which is a *different* knob from
        // this one.
        let config = PlatformConfig::from_source(source(&[(
            "KAFKA_BOOTSTRAP_SERVERS",
            "should-not-apply:9092",
        )]))
        .unwrap();
        assert_eq!(config.kafka_bootstrap_servers, "localhost:9092");
    }

    #[test]
    fn default_num_partitions_below_one_is_rejected() {
        let err = PlatformConfig::from_source(source(&[("MESHED_DEFAULT_NUM_PARTITIONS", "0")]))
            .unwrap_err();
        assert_eq!(
            err,
            ConfigError::OutOfRange("MESHED_DEFAULT_NUM_PARTITIONS", 0)
        );
    }

    #[test]
    fn default_replication_factor_below_one_is_rejected() {
        let err =
            PlatformConfig::from_source(source(&[("MESHED_DEFAULT_REPLICATION_FACTOR", "-1")]))
                .unwrap_err();
        assert_eq!(
            err,
            ConfigError::OutOfRange("MESHED_DEFAULT_REPLICATION_FACTOR", -1)
        );
    }

    #[test]
    fn default_retention_ms_below_one_is_rejected() {
        let err = PlatformConfig::from_source(source(&[("MESHED_DEFAULT_RETENTION_MS", "0")]))
            .unwrap_err();
        assert_eq!(
            err,
            ConfigError::OutOfRange("MESHED_DEFAULT_RETENTION_MS", 0)
        );
    }

    #[test]
    fn non_integer_value_is_rejected() {
        let err = PlatformConfig::from_source(source(&[(
            "MESHED_DEFAULT_NUM_PARTITIONS",
            "not-a-number",
        )]))
        .unwrap_err();
        assert_eq!(
            err,
            ConfigError::InvalidInt("MESHED_DEFAULT_NUM_PARTITIONS", "not-a-number".to_string())
        );
    }

    #[test]
    fn from_env_reads_the_real_process_environment() {
        // A light smoke test for the from_env() -> from_source() wiring
        // itself; doesn't assert on any particular env var value since
        // the real process environment is shared with the rest of the
        // test binary.
        let config = PlatformConfig::from_env().unwrap();
        assert!(!config.kafka_bootstrap_servers.is_empty());
    }
}

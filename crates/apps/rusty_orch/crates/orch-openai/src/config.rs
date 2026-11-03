//! Validated adapter settings.

use std::fmt;
use std::time::Duration;

use rusty_http::Url;

/// How to reach one OpenAI-compatible server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    base: Url,
    model: String,
    api_key_env: Option<String>,
    timeout: Duration,
    json_mode: bool,
}

/// Why a [`Config`] was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    BadUrl(String),
    /// TLS is not supported in v0.
    Https,
    EmptyModel,
    BadKeyEnv(String),
    ZeroTimeout,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadUrl(e) => write!(f, "base_url: {e}"),
            Self::Https => f.write_str(
                "base_url: https is not supported yet; use plain http to a loopback or private address",
            ),
            Self::EmptyModel => f.write_str("model: must not be empty"),
            Self::BadKeyEnv(name) => {
                write!(f, "api_key_env: {name:?} is not an environment variable name")
            }
            Self::ZeroTimeout => f.write_str("timeout: must be more than zero"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Validate settings. `base_url` is the API root, e.g.
    /// `http://baileyai:8080/v1`; requests go to `{base_url}/chat/completions`.
    pub fn new(
        base_url: &str,
        model: &str,
        api_key_env: Option<&str>,
        timeout: Duration,
        json_mode: bool,
    ) -> Result<Self, ConfigError> {
        // Only visible ASCII: the path becomes the request-target verbatim,
        // so a space or CR/LF would split the request line.
        if let Some(b) = base_url.bytes().find(|b| !(0x21..=0x7e).contains(b)) {
            return Err(ConfigError::BadUrl(format!(
                "byte 0x{b:02x} is not allowed; percent-encode it"
            )));
        }
        let base = Url::parse(base_url).map_err(|e| ConfigError::BadUrl(e.to_string()))?;
        if base.scheme != "http" {
            return Err(ConfigError::Https);
        }
        if model.trim().is_empty() {
            return Err(ConfigError::EmptyModel);
        }
        if let Some(name) = api_key_env {
            if !is_env_name(name) {
                return Err(ConfigError::BadKeyEnv(name.to_owned()));
            }
        }
        if timeout.is_zero() {
            return Err(ConfigError::ZeroTimeout);
        }
        Ok(Self {
            base,
            model: model.to_owned(),
            api_key_env: api_key_env.map(str::to_owned),
            timeout,
            json_mode,
        })
    }

    /// The parsed API root.
    pub fn base(&self) -> &Url {
        &self.base
    }

    /// The model name sent with each request.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// The variable holding the API key, if the server needs one.
    pub fn api_key_env(&self) -> Option<&str> {
        self.api_key_env.as_deref()
    }

    /// The whole-call deadline: resolve, connect, send and read.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Whether to ask for `response_format: json_object`.
    pub fn json_mode(&self) -> bool {
        self.json_mode
    }

    /// The request-target for chat completions under the base path.
    pub fn completions_target(&self) -> String {
        format!("{}/chat/completions", self.base.path.trim_end_matches('/'))
    }

    /// The host to resolve: the URL host without IPv6 brackets.
    pub fn resolve_host(&self) -> &str {
        self.base.host.trim_start_matches('[').trim_end_matches(']')
    }
}

fn is_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'_'))
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECS: Duration = Duration::from_secs(30);

    #[test]
    fn plain_http_with_a_path_is_accepted() {
        let c = Config::new("http://baileyai:8080/v1/", "qwen", None, SECS, true).expect("ok");
        assert_eq!(c.completions_target(), "/v1/chat/completions");
        assert_eq!(c.resolve_host(), "baileyai");
        assert_eq!(c.base().port, 8080);
        let v6 = Config::new("http://[::1]:8080/v1", "m", None, SECS, false).expect("ok");
        assert_eq!(v6.resolve_host(), "::1");
    }

    #[test]
    fn control_characters_and_spaces_are_refused() {
        for url in [
            "http://127.0.0.1/v1 HTTP/1.1\r\nX-Injected: yes\r\n\r\nPOST /",
            "http://127.0.0.1/v 1",
            "http://127.0.0.1/v1\t",
            "http://127.0.0.1/v\u{e9}",
        ] {
            let e = Config::new(url, "m", None, SECS, false).expect_err(url);
            assert!(matches!(e, ConfigError::BadUrl(_)), "{url}: {e:?}");
        }
    }

    #[test]
    fn refusals_name_the_field() {
        let cases = [
            ("https://host/v1", "m", None, SECS, ConfigError::Https),
            ("http://host/v1", " ", None, SECS, ConfigError::EmptyModel),
            (
                "http://host/v1",
                "m",
                Some("1KEY"),
                SECS,
                ConfigError::BadKeyEnv("1KEY".into()),
            ),
            (
                "http://host/v1",
                "m",
                None,
                Duration::ZERO,
                ConfigError::ZeroTimeout,
            ),
        ];
        for (url, model, key, timeout, expected) in cases {
            assert_eq!(
                Config::new(url, model, key, timeout, false),
                Err(expected.clone())
            );
        }
        assert!(matches!(
            Config::new("ftp://host", "m", None, SECS, false),
            Err(ConfigError::BadUrl(_))
        ));
        assert!(ConfigError::Https.to_string().starts_with("base_url:"));
    }
}

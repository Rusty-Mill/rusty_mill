//! Security scheme declarations (spec Section 4.5, proto `SecurityScheme`
//! and friends). Modeled on the OpenAPI 3.2 Security Scheme Object.

use std::collections::HashMap;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// A discriminated union of the supported authentication mechanisms
/// (proto `SecurityScheme`, oneof `scheme`).
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum SecurityScheme {
    ApiKey {
        #[serde(rename = "apiKeySecurityScheme")]
        api_key_security_scheme: ApiKeySecurityScheme,
    },
    HttpAuth {
        #[serde(rename = "httpAuthSecurityScheme")]
        http_auth_security_scheme: HttpAuthSecurityScheme,
    },
    OAuth2 {
        #[serde(rename = "oauth2SecurityScheme")]
        oauth2_security_scheme: OAuth2SecurityScheme,
    },
    OpenIdConnect {
        #[serde(rename = "openIdConnectSecurityScheme")]
        open_id_connect_security_scheme: OpenIdConnectSecurityScheme,
    },
    MutualTls {
        #[serde(rename = "mtlsSecurityScheme")]
        mtls_security_scheme: MutualTlsSecurityScheme,
    },
}

/// See `PartContentRepr` (`src/types/message.rs`) for why this mirror type
/// - and the manual `Deserialize` impl below that delegates to it only
/// after checking exactly one key is present - exists: spec Section 4.5.1
/// ("A SecurityScheme MUST contain exactly one of the following:
/// apiKeySecurityScheme, httpAuthSecurityScheme, oauth2SecurityScheme,
/// openIdConnectSecurityScheme, mtlsSecurityScheme") isn't enforced by a
/// plain derived untagged deserialize.
#[derive(Deserialize)]
#[serde(untagged)]
enum SecuritySchemeRepr {
    ApiKey {
        #[serde(rename = "apiKeySecurityScheme")]
        api_key_security_scheme: ApiKeySecurityScheme,
    },
    HttpAuth {
        #[serde(rename = "httpAuthSecurityScheme")]
        http_auth_security_scheme: HttpAuthSecurityScheme,
    },
    OAuth2 {
        #[serde(rename = "oauth2SecurityScheme")]
        oauth2_security_scheme: OAuth2SecurityScheme,
    },
    OpenIdConnect {
        #[serde(rename = "openIdConnectSecurityScheme")]
        open_id_connect_security_scheme: OpenIdConnectSecurityScheme,
    },
    MutualTls {
        #[serde(rename = "mtlsSecurityScheme")]
        mtls_security_scheme: MutualTlsSecurityScheme,
    },
}

impl From<SecuritySchemeRepr> for SecurityScheme {
    fn from(repr: SecuritySchemeRepr) -> Self {
        match repr {
            SecuritySchemeRepr::ApiKey {
                api_key_security_scheme,
            } => SecurityScheme::ApiKey {
                api_key_security_scheme,
            },
            SecuritySchemeRepr::HttpAuth {
                http_auth_security_scheme,
            } => SecurityScheme::HttpAuth {
                http_auth_security_scheme,
            },
            SecuritySchemeRepr::OAuth2 {
                oauth2_security_scheme,
            } => SecurityScheme::OAuth2 {
                oauth2_security_scheme,
            },
            SecuritySchemeRepr::OpenIdConnect {
                open_id_connect_security_scheme,
            } => SecurityScheme::OpenIdConnect {
                open_id_connect_security_scheme,
            },
            SecuritySchemeRepr::MutualTls { mtls_security_scheme } => {
                SecurityScheme::MutualTls { mtls_security_scheme }
            }
        }
    }
}

impl<'de> Deserialize<'de> for SecurityScheme {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let obj = value
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("a SecurityScheme must be a JSON object"))?;
        const KEYS: [&str; 5] = [
            "apiKeySecurityScheme",
            "httpAuthSecurityScheme",
            "oauth2SecurityScheme",
            "openIdConnectSecurityScheme",
            "mtlsSecurityScheme",
        ];
        let present: Vec<&str> = KEYS.iter().copied().filter(|k| obj.contains_key(*k)).collect();
        if present.len() != 1 {
            return Err(serde::de::Error::custom(format!(
                "a SecurityScheme must contain exactly one of {KEYS:?} (spec Section 4.5.1); \
                 found {} ({present:?})",
                present.len()
            )));
        }
        serde_json::from_value::<SecuritySchemeRepr>(value)
            .map(Into::into)
            .map_err(serde::de::Error::custom)
    }
}

/// API key-based authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeySecurityScheme {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    /// Valid values: `"query"`, `"header"`, or `"cookie"`.
    pub location: String,
    pub name: String,
}

/// HTTP authentication (Basic, Bearer, etc).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpAuthSecurityScheme {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    /// An IANA-registered HTTP authentication scheme name, e.g. `"Bearer"`.
    pub scheme: String,
    #[serde(rename = "bearerFormat", skip_serializing_if = "Option::is_none", default)]
    pub bearer_format: Option<String>,
}

/// OAuth 2.0 authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2SecurityScheme {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    pub flows: OAuthFlows,
    #[serde(
        rename = "oauth2MetadataUrl",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub oauth2_metadata_url: Option<String>,
}

/// OpenID Connect authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenIdConnectSecurityScheme {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    #[serde(rename = "openIdConnectUrl")]
    pub open_id_connect_url: String,
}

/// Mutual TLS authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutualTlsSecurityScheme {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
}

/// The supported OAuth 2.0 flows (proto `OAuthFlows`, oneof `flow`).
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum OAuthFlows {
    AuthorizationCode {
        #[serde(rename = "authorizationCode")]
        authorization_code: AuthorizationCodeOAuthFlow,
    },
    ClientCredentials {
        #[serde(rename = "clientCredentials")]
        client_credentials: ClientCredentialsOAuthFlow,
    },
    /// Deprecated: use Authorization Code + PKCE instead.
    Implicit { implicit: ImplicitOAuthFlow },
    /// Deprecated: use Authorization Code + PKCE or Device Code.
    Password { password: PasswordOAuthFlow },
    DeviceCode {
        #[serde(rename = "deviceCode")]
        device_code: DeviceCodeOAuthFlow,
    },
}

/// See `PartContentRepr` (`src/types/message.rs`) - spec Section 4.5.7 ("A
/// OAuthFlows MUST contain exactly one of the following: authorizationCode,
/// clientCredentials, implicit, password, deviceCode") isn't enforced by a
/// plain derived untagged deserialize.
#[derive(Deserialize)]
#[serde(untagged)]
enum OAuthFlowsRepr {
    AuthorizationCode {
        #[serde(rename = "authorizationCode")]
        authorization_code: AuthorizationCodeOAuthFlow,
    },
    ClientCredentials {
        #[serde(rename = "clientCredentials")]
        client_credentials: ClientCredentialsOAuthFlow,
    },
    Implicit {
        implicit: ImplicitOAuthFlow,
    },
    Password {
        password: PasswordOAuthFlow,
    },
    DeviceCode {
        #[serde(rename = "deviceCode")]
        device_code: DeviceCodeOAuthFlow,
    },
}

impl From<OAuthFlowsRepr> for OAuthFlows {
    fn from(repr: OAuthFlowsRepr) -> Self {
        match repr {
            OAuthFlowsRepr::AuthorizationCode { authorization_code } => {
                OAuthFlows::AuthorizationCode { authorization_code }
            }
            OAuthFlowsRepr::ClientCredentials { client_credentials } => {
                OAuthFlows::ClientCredentials { client_credentials }
            }
            OAuthFlowsRepr::Implicit { implicit } => OAuthFlows::Implicit { implicit },
            OAuthFlowsRepr::Password { password } => OAuthFlows::Password { password },
            OAuthFlowsRepr::DeviceCode { device_code } => OAuthFlows::DeviceCode { device_code },
        }
    }
}

impl<'de> Deserialize<'de> for OAuthFlows {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let obj = value
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("an OAuthFlows must be a JSON object"))?;
        const KEYS: [&str; 5] = [
            "authorizationCode",
            "clientCredentials",
            "implicit",
            "password",
            "deviceCode",
        ];
        let present: Vec<&str> = KEYS.iter().copied().filter(|k| obj.contains_key(*k)).collect();
        if present.len() != 1 {
            return Err(serde::de::Error::custom(format!(
                "an OAuthFlows must contain exactly one of {KEYS:?} (spec Section 4.5.7); found \
                 {} ({present:?})",
                present.len()
            )));
        }
        serde_json::from_value::<OAuthFlowsRepr>(value)
            .map(Into::into)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationCodeOAuthFlow {
    #[serde(rename = "authorizationUrl")]
    pub authorization_url: String,
    #[serde(rename = "tokenUrl")]
    pub token_url: String,
    #[serde(rename = "refreshUrl", skip_serializing_if = "Option::is_none", default)]
    pub refresh_url: Option<String>,
    pub scopes: HashMap<String, String>,
    /// PKCE (RFC 7636) should always be used for public clients and is
    /// recommended for all clients.
    #[serde(rename = "pkceRequired", default)]
    pub pkce_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientCredentialsOAuthFlow {
    #[serde(rename = "tokenUrl")]
    pub token_url: String,
    #[serde(rename = "refreshUrl", skip_serializing_if = "Option::is_none", default)]
    pub refresh_url: Option<String>,
    pub scopes: HashMap<String, String>,
}

/// Deprecated: use Authorization Code + PKCE instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplicitOAuthFlow {
    #[serde(rename = "authorizationUrl")]
    pub authorization_url: String,
    #[serde(rename = "refreshUrl", skip_serializing_if = "Option::is_none", default)]
    pub refresh_url: Option<String>,
    #[serde(default)]
    pub scopes: HashMap<String, String>,
}

/// Deprecated: use Authorization Code + PKCE or Device Code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasswordOAuthFlow {
    #[serde(rename = "tokenUrl")]
    pub token_url: String,
    #[serde(rename = "refreshUrl", skip_serializing_if = "Option::is_none", default)]
    pub refresh_url: Option<String>,
    #[serde(default)]
    pub scopes: HashMap<String, String>,
}

/// OAuth 2.0 Device Code flow (RFC 8628), for input-constrained devices.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCodeOAuthFlow {
    #[serde(rename = "deviceAuthorizationUrl")]
    pub device_authorization_url: String,
    #[serde(rename = "tokenUrl")]
    pub token_url: String,
    #[serde(rename = "refreshUrl", skip_serializing_if = "Option::is_none", default)]
    pub refresh_url: Option<String>,
    pub scopes: HashMap<String, String>,
}

/// A list of required scopes for one security scheme.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StringList {
    #[serde(default)]
    pub list: Vec<String>,
}

/// The security schemes (by name) required to contact an agent or invoke a
/// skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityRequirement {
    #[serde(default)]
    pub schemes: HashMap<String, StringList>,
}

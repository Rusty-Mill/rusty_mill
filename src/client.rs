//! OAuth client identity and authentication (RFC 6749 §2.3, RFC 7523).

use crate::encoding::base64::encode_standard;
use crate::encoding::percent::encode as percent_encode;
use std::fmt;

/// A client identifier (RFC 6749 §2.2). Not secret; safe to embed in
/// public clients (SPAs, native/mobile apps, CLI tools).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClientId(String);

impl ClientId {
    pub fn new(id: impl Into<String>) -> Self {
        ClientId(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ClientId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A client secret (RFC 6749 §2.3.1). Confidential -- its `Debug` impl
/// redacts the value so it doesn't leak into logs or panics.
#[derive(Clone, PartialEq, Eq)]
pub struct ClientSecret(String);

impl ClientSecret {
    pub fn new(secret: impl Into<String>) -> Self {
        ClientSecret(secret.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ClientSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ClientSecret").field(&"[redacted]").finish()
    }
}

/// How the client authenticates to the token endpoint, per RFC 6749
/// §2.3.1 and the JWT bearer client-assertion methods of RFC 7523 §2.2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMethod {
    /// No client authentication -- for public clients (RFC 6749 §2.1)
    /// that cannot hold a secret, such as native apps or SPAs. PKCE
    /// (RFC 7636) is required to secure these under OAuth 2.1.
    None,
    /// `client_secret_basic`: HTTP Basic authentication with the client
    /// ID/secret (RFC 6749 §2.3.1). The server MUST support this method;
    /// prefer it over `client_secret_post` unless the server requires
    /// otherwise.
    ClientSecretBasic,
    /// `client_secret_post`: client ID/secret sent as body parameters.
    ClientSecretPost,
    /// `client_secret_jwt` (RFC 7523 / OIDC Core §9): a JWT assertion
    /// signed with HS256 using the client secret as the HMAC key.
    ClientSecretJwt,
    /// `private_key_jwt` (RFC 7523 / OIDC Core §9): a JWT assertion
    /// signed with an asymmetric key registered with the server.
    PrivateKeyJwt,
    /// `tls_client_auth` (RFC 8705 §2.1): the client authenticates by
    /// presenting an X.509 certificate matching one registered with the
    /// server (via a subject DN, SAN, etc.), verified at the TLS layer.
    /// Since this crate never performs TLS itself (see the crate-level
    /// docs), the certificate presentation and verification is entirely
    /// the caller's HTTP client's responsibility; this variant only
    /// controls how the request *body* is shaped -- `client_id` is sent,
    /// but never a `client_secret`, since the certificate is the proof of
    /// identity.
    TlsClientAuth,
    /// `self_signed_tls_client_auth` (RFC 8705 §2.2): like
    /// `TlsClientAuth`, but the server trusts a specific self-signed
    /// certificate registered out-of-band rather than a CA-issued one.
    /// Produces an identical request body to `TlsClientAuth`; the
    /// distinction only matters to the server's own certificate
    /// validation policy.
    SelfSignedTlsClientAuth,
}

/// An OAuth client's identity and authentication configuration.
#[derive(Debug, Clone)]
pub struct Client {
    pub client_id: ClientId,
    pub client_secret: Option<ClientSecret>,
    pub auth_method: AuthMethod,
    /// A pre-signed JWT client assertion, required when `auth_method` is
    /// [`AuthMethod::PrivateKeyJwt`] (RFC 7523 §2.2). This crate does not
    /// perform private-key signing itself (see the crate-level docs on
    /// RSA verify-only), so the caller must sign the assertion with
    /// whatever key material and library they trust and hand it over via
    /// [`Client::with_client_assertion`]. Not used for
    /// [`AuthMethod::ClientSecretJwt`], which this crate signs itself
    /// with HS256 using `client_secret`.
    pub client_assertion: Option<String>,
}

impl Client {
    /// A public client: no secret, no client authentication. Always pair
    /// with PKCE for the authorization code grant.
    pub fn public(client_id: ClientId) -> Self {
        Client {
            client_id,
            client_secret: None,
            auth_method: AuthMethod::None,
            client_assertion: None,
        }
    }

    /// A confidential client authenticating with `client_secret_basic`
    /// (the method every RFC 6749-compliant server must support).
    pub fn confidential(client_id: ClientId, client_secret: ClientSecret) -> Self {
        Client {
            client_id,
            client_secret: Some(client_secret),
            auth_method: AuthMethod::ClientSecretBasic,
            client_assertion: None,
        }
    }

    /// Overrides the authentication method (e.g. to `client_secret_post`,
    /// or one of the JWT-assertion methods).
    pub fn with_auth_method(mut self, method: AuthMethod) -> Self {
        self.auth_method = method;
        self
    }

    /// Sets a pre-signed JWT client assertion for `private_key_jwt`
    /// authentication (RFC 7523 §2.2). Ignored unless `auth_method` is
    /// [`AuthMethod::PrivateKeyJwt`].
    pub fn with_client_assertion(mut self, assertion: impl Into<String>) -> Self {
        self.client_assertion = Some(assertion.into());
        self
    }

    /// Builds the `Authorization: Basic ...` header value for
    /// `client_secret_basic` authentication (RFC 6749 §2.3.1): the client
    /// ID and secret are each individually form-urlencoded, joined with
    /// `:`, then Base64-encoded.
    pub fn basic_auth_header(&self) -> Option<String> {
        let secret = self.client_secret.as_ref()?;
        let credentials = format!(
            "{}:{}",
            percent_encode(self.client_id.as_str()),
            percent_encode(secret.as_str())
        );
        Some(format!("Basic {}", encode_standard(credentials.as_bytes())))
    }

    /// Builds the `client_assertion_type`/`client_assertion` parameter
    /// pair for `client_secret_jwt` / `private_key_jwt` authentication
    /// (RFC 7523 §2.2), if this client uses either method. Returns `None`
    /// for every other [`AuthMethod`], and the pair's second element is
    /// always [`JWT_BEARER_CLIENT_ASSERTION_TYPE`].
    ///
    /// For `client_secret_jwt`, the assertion is generated fresh on every
    /// call (a new `jti`/`exp`, bound to `token_endpoint` as `aud`) using
    /// HS256 over `client_secret` -- this crate already implements HMAC
    /// signing, so there's no reason to make the caller do it. For
    /// `private_key_jwt`, `client_assertion` must already be set via
    /// [`Client::with_client_assertion`]; this crate does not perform
    /// private-key signing.
    pub(crate) fn build_client_assertion(
        &self,
        token_endpoint: &str,
    ) -> crate::error::Result<Option<(&'static str, String)>> {
        match self.auth_method {
            AuthMethod::ClientSecretJwt => {
                let secret = self.client_secret.as_ref().ok_or_else(|| {
                    crate::error::Error::Validation(
                        "client_secret_jwt requires a client secret".to_string(),
                    )
                })?;
                let now = crate::jwt::now_unix();
                let jti = crate::encoding::base64::encode_url_safe_no_pad(
                    &crate::rand::random_bytes(16)?,
                );
                let claims = crate::json::Value::object([
                    (
                        "iss".to_string(),
                        crate::json::Value::from(self.client_id.as_str()),
                    ),
                    (
                        "sub".to_string(),
                        crate::json::Value::from(self.client_id.as_str()),
                    ),
                    ("aud".to_string(), crate::json::Value::from(token_endpoint)),
                    ("exp".to_string(), crate::json::Value::from(now + 60)),
                    ("iat".to_string(), crate::json::Value::from(now)),
                    ("jti".to_string(), crate::json::Value::from(jti)),
                ]);
                let assertion = crate::jwt::encode_hs256(&claims, secret.as_str().as_bytes(), &[]);
                Ok(Some((JWT_BEARER_CLIENT_ASSERTION_TYPE, assertion)))
            }
            AuthMethod::PrivateKeyJwt => {
                let assertion = self.client_assertion.clone().ok_or_else(|| {
                    crate::error::Error::Validation(
                        "private_key_jwt requires a pre-signed assertion set via \
                         Client::with_client_assertion (this crate does not perform \
                         private-key signing)"
                            .to_string(),
                    )
                })?;
                Ok(Some((JWT_BEARER_CLIENT_ASSERTION_TYPE, assertion)))
            }
            _ => Ok(None),
        }
    }
}

/// RFC 7523 §2.2: the `client_assertion_type` value for JWT bearer client
/// authentication.
pub const JWT_BEARER_CLIENT_ASSERTION_TYPE: &str =
    "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_auth_header_matches_rfc_example_shape() {
        let client = Client::confidential(
            ClientId::new("s6BhdRkqt3"),
            ClientSecret::new("7Fjfp0ZBr1KtDRbnfVdmIw"),
        );
        let header = client.basic_auth_header().unwrap();
        assert!(header.starts_with("Basic "));
        let decoded = crate::encoding::base64::decode_standard(&header[6..]).unwrap();
        assert_eq!(
            String::from_utf8(decoded).unwrap(),
            "s6BhdRkqt3:7Fjfp0ZBr1KtDRbnfVdmIw"
        );
    }

    #[test]
    fn public_client_has_no_basic_header() {
        let client = Client::public(ClientId::new("public-app"));
        assert!(client.basic_auth_header().is_none());
    }

    #[test]
    fn client_secret_debug_is_redacted() {
        let secret = ClientSecret::new("super-secret-value");
        let debug = format!("{:?}", secret);
        assert!(!debug.contains("super-secret-value"));
    }
}

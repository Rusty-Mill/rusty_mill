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
}

/// An OAuth client's identity and authentication configuration.
#[derive(Debug, Clone)]
pub struct Client {
    pub client_id: ClientId,
    pub client_secret: Option<ClientSecret>,
    pub auth_method: AuthMethod,
}

impl Client {
    /// A public client: no secret, no client authentication. Always pair
    /// with PKCE for the authorization code grant.
    pub fn public(client_id: ClientId) -> Self {
        Client {
            client_id,
            client_secret: None,
            auth_method: AuthMethod::None,
        }
    }

    /// A confidential client authenticating with `client_secret_basic`
    /// (the method every RFC 6749-compliant server must support).
    pub fn confidential(client_id: ClientId, client_secret: ClientSecret) -> Self {
        Client {
            client_id,
            client_secret: Some(client_secret),
            auth_method: AuthMethod::ClientSecretBasic,
        }
    }

    /// Overrides the authentication method (e.g. to `client_secret_post`,
    /// or one of the JWT-assertion methods).
    pub fn with_auth_method(mut self, method: AuthMethod) -> Self {
        self.auth_method = method;
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
}

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

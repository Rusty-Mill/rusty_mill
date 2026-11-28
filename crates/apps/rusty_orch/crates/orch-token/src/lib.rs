//! The token orch mints for each jailed agent session (Let Em Loose #1382).
//!
//! A short-lived ES256 JWT scoped to one project, task, claim and lens
//! (726-sandbox-design §2, D180). The gateway checks it against a file JWKS
//! (`agentgateway-auth` `FileJwks`) and selects the lens policy from
//! `lel.lens` and `lel.policy`. The two share only the claims and the JWKS, so
//! neither crate imports the other (D102).
//!
//! Sans-IO (D209): the caller reads the key file, reads the clock and draws
//! the 128-bit `jti`, then passes them in. Nothing here touches the disk, the
//! clock or a random source, and the token is never logged.
//!
//! ```text
//! let key = SigningKey::from_pem(&pem_bytes, "orch-2026-11")?;
//! let claims = Claims::new(host, scope, policy_digest(&policy_bytes), grants, jti, now, budget)?;
//! let token = mint(&claims, &key)?;   // goes into the jail.json token file
//! let entry = jwk(&key)?;             // goes into the gateway's JWKS file
//! ```

use std::fmt;
use std::time::Duration;

use jsonwebtoken::jwk::{Jwk, PublicKeyUse};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::{json, Map, Value};

/// The only audience the gateway accepts (§2).
pub const AUDIENCE: &str = "lel-gateway";

/// Added to the session budget so a session that runs to its limit still
/// holds a valid token while it winds down (§2).
pub const GRACE: Duration = Duration::from_secs(5 * 60);

/// No token lives longer than this, whatever the budget (§2). A longer
/// session gets a fresh token with the same `sub`.
pub const MAX_LIFETIME: Duration = Duration::from_secs(4 * 60 * 60);

/// Why a token could not be built or signed.
#[derive(Debug)]
pub enum TokenError {
    /// A required field was empty.
    Empty(&'static str),
    /// A field that is part of `sub` held the `/` separator.
    Separator(&'static str),
    /// `now` plus the lifetime does not fit in a Unix timestamp.
    ClockOverflow,
    /// The key is not a P-256 private key in PKCS#8 PEM.
    Key(jsonwebtoken::errors::Error),
    /// Signing failed.
    Sign(jsonwebtoken::errors::Error),
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(field) => write!(f, "`{field}` must not be empty"),
            Self::Separator(field) => write!(f, "`{field}` must not contain `/`"),
            Self::ClockOverflow => f.write_str("token expiry overflows a Unix timestamp"),
            Self::Key(err) => write!(f, "signing key is not a P-256 PKCS#8 PEM key: {err}"),
            Self::Sign(err) => write!(f, "signing the token failed: {err}"),
        }
    }
}

impl std::error::Error for TokenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Key(err) | Self::Sign(err) => Some(err),
            _ => None,
        }
    }
}

/// What the session may do: the lens's `role` (§5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Writes the workspace.
    Builder,
    /// Reads and runs, but writes nothing.
    Critic,
}

impl Role {
    /// The claim value, as lens policy files spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Builder => "builder",
            Self::Critic => "critic",
        }
    }
}

/// The sha256 of a lens policy file's bytes, lowercase hex. Only
/// [`policy_digest`] makes one, so `lel.policy` always holds a digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyDigest(String);

impl PolicyDigest {
    /// The hex digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Digest a lens policy file for `lel.policy`, exactly as the gateway's lens
/// policy check does, which binds the token to the lens config (§5).
pub fn policy_digest(bytes: &[u8]) -> PolicyDigest {
    PolicyDigest(rusty_hex::encode(&rusty_rsa::sha256(bytes)))
}

/// Where the session works and as what. Every id is non-empty; those that
/// make up `sub` hold no `/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    project: String,
    task: String,
    claim: String,
    lens: String,
    role: Role,
    session: u32,
}

impl Scope {
    /// Check and hold the session's scope. `session` is the session's number
    /// within the claim and lens, the `<n>` of `sub`.
    pub fn new(
        project: &str,
        task: &str,
        claim: &str,
        lens: &str,
        role: Role,
        session: u32,
    ) -> Result<Self, TokenError> {
        Ok(Self {
            project: sub_part("lel.project", project)?,
            task: non_empty("lel.task", task)?,
            claim: sub_part("lel.claim", claim)?,
            lens: sub_part("lel.lens", lens)?,
            role,
            session,
        })
    }

    /// The agent session's subject: `<project>/<claim>/<lens>/<n>` (§2).
    pub fn subject(&self) -> String {
        format!(
            "{}/{}/{}/{}",
            self.project, self.claim, self.lens, self.session
        )
    }
}

/// The checked claims of one token. Build with [`Claims::new`]; `exp` is
/// always after `iat` and within [`MAX_LIFETIME`] of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claims {
    issuer: String,
    scope: Scope,
    policy: PolicyDigest,
    grants: Vec<String>,
    jti: [u8; 16],
    issued_at: u64,
    expires_at: u64,
}

impl Claims {
    /// Claims for a session starting at `now` (Unix seconds) with the given
    /// wall-clock `budget`. `host` names the orch host in `iss`
    /// (`lel-orch@<host>`); `jti` is a fresh random 128-bit id, the
    /// revocation key; `grants` are the widening grants Nano approved (§5.3),
    /// usually none.
    pub fn new(
        host: &str,
        scope: Scope,
        policy: PolicyDigest,
        grants: Vec<String>,
        jti: [u8; 16],
        now: u64,
        budget: Duration,
    ) -> Result<Self, TokenError> {
        let host = non_empty("iss host", host)?;
        if grants.iter().any(String::is_empty) {
            return Err(TokenError::Empty("lel.grants entry"));
        }
        let expires_at = now
            .checked_add(lifetime(budget).as_secs())
            .ok_or(TokenError::ClockOverflow)?;
        Ok(Self {
            issuer: format!("lel-orch@{host}"),
            scope,
            policy,
            grants,
            jti,
            issued_at: now,
            expires_at,
        })
    }

    /// `jti` as lowercase hex: safe to log, and what revocation names.
    pub fn jti(&self) -> String {
        rusty_hex::encode(&self.jti)
    }

    /// `sub`: safe to log.
    pub fn subject(&self) -> String {
        self.scope.subject()
    }

    /// `exp`, Unix seconds.
    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }

    /// The JWT payload. Dotted names are literal top-level keys, as the
    /// gateway reads them. `lel.grants` is left out when empty.
    fn to_json(&self) -> Value {
        let s = &self.scope;
        let mut claims = Map::new();
        for (key, value) in [
            ("iss", json!(self.issuer)),
            ("aud", json!(AUDIENCE)),
            ("sub", json!(s.subject())),
            ("jti", json!(self.jti())),
            ("iat", json!(self.issued_at)),
            ("exp", json!(self.expires_at)),
            ("lel.project", json!(s.project)),
            ("lel.task", json!(s.task)),
            ("lel.claim", json!(s.claim)),
            ("lel.lens", json!(s.lens)),
            ("lel.role", json!(s.role.as_str())),
            ("lel.policy", json!(self.policy.as_str())),
        ] {
            claims.insert(key.to_owned(), value);
        }
        if !self.grants.is_empty() {
            claims.insert("lel.grants".to_owned(), json!(self.grants));
        }
        Value::Object(claims)
    }
}

/// orch's ES256 signing key and its `kid`. Debug output shows the `kid` only.
pub struct SigningKey {
    key: EncodingKey,
    kid: String,
}

impl fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SigningKey")
            .field("kid", &self.kid)
            .finish_non_exhaustive()
    }
}

impl SigningKey {
    /// Parse a P-256 private key in PKCS#8 PEM, as
    /// `openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256` writes
    /// it. Any other curve is refused here rather than at the first mint.
    pub fn from_pem(pem: &[u8], kid: &str) -> Result<Self, TokenError> {
        let kid = non_empty("kid", kid)?;
        let key = EncodingKey::from_ec_pem(pem).map_err(TokenError::Key)?;
        let signing = Self { key, kid };
        public_jwk(&signing)?;
        Ok(signing)
    }

    /// The key id the token header and the JWKS entry carry.
    pub fn kid(&self) -> &str {
        &self.kid
    }
}

/// The token's lifetime for a session budget: budget plus [`GRACE`], capped at
/// [`MAX_LIFETIME`].
pub fn lifetime(budget: Duration) -> Duration {
    budget.saturating_add(GRACE).min(MAX_LIFETIME)
}

/// Sign `claims` as an ES256 JWT whose header names the key's `kid`.
pub fn mint(claims: &Claims, key: &SigningKey) -> Result<String, TokenError> {
    let mut header = Header::new(Algorithm::ES256);
    header.kid = Some(key.kid.clone());
    encode(&header, &claims.to_json(), &key.key).map_err(TokenError::Sign)
}

/// The public JWKS entry for `key`, for the gateway's JWKS file: EC P-256,
/// `alg` ES256, `use` sig, the key's `kid`.
pub fn jwk(key: &SigningKey) -> Result<Jwk, TokenError> {
    public_jwk(key)
}

fn public_jwk(key: &SigningKey) -> Result<Jwk, TokenError> {
    let mut jwk = Jwk::from_encoding_key(&key.key, Algorithm::ES256).map_err(TokenError::Key)?;
    jwk.common.key_id = Some(key.kid.clone());
    jwk.common.public_key_use = Some(PublicKeyUse::Signature);
    Ok(jwk)
}

fn non_empty(field: &'static str, value: &str) -> Result<String, TokenError> {
    if value.is_empty() {
        return Err(TokenError::Empty(field));
    }
    Ok(value.to_owned())
}

fn sub_part(field: &'static str, value: &str) -> Result<String, TokenError> {
    let value = non_empty(field, value)?;
    if value.contains('/') {
        return Err(TokenError::Separator(field));
    }
    Ok(value)
}

#[cfg(test)]
mod tests;

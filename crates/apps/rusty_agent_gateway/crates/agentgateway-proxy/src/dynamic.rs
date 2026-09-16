//! `dynamic`: the destination taken from the request.
//!
//! Every other backend kind names its upstream in the configuration. This one
//! does not: the route becomes a forward proxy, and the address comes out of
//! the request being served.
//!
//! # This hands the client the steering wheel
//!
//! Worth stating plainly, because it is the whole feature and also its whole
//! risk. With no `target`, a `dynamic` route dials whatever authority the
//! request carries — so anyone who can reach the listener can make the gateway
//! open a connection anywhere the gateway can reach, from the gateway's own
//! network position. That is a forward proxy, which is a legitimate thing to
//! run and a catastrophic thing to run open.
//!
//! So a route carrying one is logged at startup, loudly, and the log says what
//! to put in front of it. Refusing to serve it would be inventing a policy
//! upstream does not have; saying nothing would be worse.
//!
//! # `target` moves the decision, it does not remove it
//!
//! A `target` expression reads the client's request too. Pointing it at a
//! header does not make the value trustworthy — it makes it *someone else's*
//! choice, and only if a hop the operator trusts is the one setting that
//! header and a policy strips whatever the client sent. Upstream's own note on
//! the field says as much: the expression and the policies feeding it are
//! trusted to select the target.

use agentgateway_config::DynamicBackend;
use http::{Request, uri::Authority};
use serde_json::json;

/// A `target` expression that will not compile.
#[derive(Debug, thiserror::Error)]
#[error("{at}.target: `{source_text}` is not a valid CEL expression: {reason}")]
pub struct TargetError {
    /// Where in the configuration it came from.
    pub at: String,
    /// The expression as written.
    pub source_text: String,
    /// What the compiler said.
    pub reason: String,
}

/// Where a `dynamic` route sends one request.
#[derive(Debug)]
pub struct Dynamic {
    /// Compiled `target`, or `None` to read the request's own authority.
    target: Option<cel::Program>,
}

impl Dynamic {
    /// Compile a `dynamic` backend.
    ///
    /// An expression that does not compile is a startup failure rather than a
    /// silent fall back to the request's authority: the two choose different
    /// upstreams, and quietly picking the other one is how traffic ends up
    /// somewhere nobody asked for.
    pub fn new(backend: &DynamicBackend, at: &str) -> Result<Self, TargetError> {
        let target = match backend.target.as_deref() {
            Some(source_text) => {
                Some(
                    cel::Program::compile(source_text).map_err(|err| TargetError {
                        at: at.to_string(),
                        source_text: source_text.to_string(),
                        reason: err.to_string(),
                    })?,
                )
            }
            None => None,
        };
        Ok(Dynamic { target })
    }

    /// Whether the destination comes from an expression rather than the address
    /// the client dialled.
    pub fn is_computed(&self) -> bool {
        self.target.is_some()
    }

    /// The authority to dial for one request.
    ///
    /// `None` when there is nothing to dial — an expression that produced no
    /// string, or a request carrying no authority at all. Both are answered
    /// with a 400 rather than a guess, since the alternative is dialling
    /// somewhere the request did not name.
    pub fn authority<B>(&self, request: &Request<B>) -> Option<Authority> {
        let text = match &self.target {
            Some(program) => self.evaluate(program, request)?,
            None => authority_of(request)?,
        };
        Authority::try_from(text.as_str()).ok()
    }

    fn evaluate<B>(&self, program: &cel::Program, request: &Request<B>) -> Option<String> {
        let headers: std::collections::BTreeMap<String, String> = request
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_string(),
                    value.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect();

        let mut context = cel::Context::default();
        context
            .add_variable(
                "request",
                json!({
                    "headers": headers,
                    "method": request.method().as_str(),
                    "path": request.uri().path(),
                    "authority": authority_of(request).unwrap_or_default(),
                }),
            )
            .ok()?;

        match program.execute(&context).ok()? {
            cel::Value::String(text) => Some(text.to_string()),
            // Anything else is not an address. Rendering it would dial a
            // stringified list or a boolean, which cannot be what was meant.
            _ => None,
        }
    }
}

/// The authority a request carries, from the URI or the `Host` header.
///
/// A proxied request has it in the URI; an ordinary one has only the header.
/// Both are the client saying where it wanted to go.
fn authority_of<B>(request: &Request<B>) -> Option<String> {
    if let Some(authority) = request.uri().authority() {
        return Some(authority.to_string());
    }
    request
        .headers()
        .get(http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .filter(|host| !host.is_empty())
}

#[cfg(test)]
mod tests;

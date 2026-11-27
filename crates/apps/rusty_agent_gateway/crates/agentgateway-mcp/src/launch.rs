//! A `stdio` target's per-session launch, filled from the verified token
//! (Let Em Loose D226).
//!
//! Each session of a `per_session` target gets its own child. With a
//! `session` launch configured, that child's command, arguments and
//! environment come from the token's claims, so the gateway can start it
//! inside that session's jail. Nothing in a launch comes from the call.

use agentgateway_config::StdioLaunch;

use crate::identity::{IdentityError, Template};

/// A compiled `session` launch.
#[derive(Debug, Clone)]
pub(crate) struct Launch {
    cmd: Template,
    args: Vec<Template>,
    env: Vec<(String, Template)>,
}

/// A launch with every claim filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Resolved {
    /// Executable to run.
    pub(crate) cmd: String,
    /// Its arguments.
    pub(crate) args: Vec<String>,
    /// Extra environment, in name order.
    pub(crate) env: Vec<(String, String)>,
}

impl Resolved {
    /// The launch as one string, to tell two launches apart in a pool key.
    pub(crate) fn key(&self) -> String {
        let mut key = self.cmd.clone();
        for part in self
            .args
            .iter()
            .chain(self.env.iter().flat_map(|(k, v)| [k, v]))
        {
            key.push('\0');
            key.push_str(part);
        }
        key
    }
}

/// Whether a claim value may go into a launch: a non-empty run of
/// `[A-Za-z0-9._-]` that is not `.` or `..`. It can name a path segment but
/// never a separator, an option's `=`, or a step out of a directory.
fn allowed(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

impl Launch {
    /// Compile `config`, reporting the first bad template.
    ///
    /// # Errors
    /// [`IdentityError::Config`] naming where it is.
    pub(crate) fn new(config: &StdioLaunch, at: &str) -> Result<Self, IdentityError> {
        let cmd = Template::parse_any(&config.cmd, &format!("{at}.cmd"))?;
        let args = config
            .args
            .iter()
            .enumerate()
            .map(|(i, arg)| Template::parse_any(arg, &format!("{at}.args[{i}]")))
            .collect::<Result<_, _>>()?;
        let env = config
            .env
            .iter()
            .map(|(name, value)| {
                let template = Template::parse_any(value, &format!("{at}.env.{name}"))?;
                Ok((name.clone(), template))
            })
            .collect::<Result<_, IdentityError>>()?;
        Ok(Launch { cmd, args, env })
    }

    /// Fill every template from `claims`.
    ///
    /// # Errors
    /// [`IdentityError::Unresolved`] for a missing claim, or
    /// [`IdentityError::Value`] for one that may not go into a launch.
    pub(crate) fn resolve(
        &self,
        claims: Option<&serde_json::Value>,
    ) -> Result<Resolved, IdentityError> {
        let fill = |t: &Template| t.resolve_checked(claims, allowed);
        Ok(Resolved {
            cmd: fill(&self.cmd)?,
            args: self.args.iter().map(fill).collect::<Result<_, _>>()?,
            env: self
                .env
                .iter()
                .map(|(name, t)| Ok((name.clone(), fill(t)?)))
                .collect::<Result<_, IdentityError>>()?,
        })
    }
}

#[cfg(test)]
mod tests;

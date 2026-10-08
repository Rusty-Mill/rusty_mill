//! The part of RFC 6570 that resource templates use in practice: `{name}`
//! (one path segment: anything but `/`, `?` and `#`) and `{+name}` (reserved
//! expansion: anything, slashes included). Every other operator is refused
//! when the template is registered, instead of being half-supported.

use std::fmt;

/// A compiled template.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UriTemplate {
    parts: Vec<Part>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    Literal(String),
    Var { name: String, reserved: bool },
}

/// Why a template could not be compiled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateError(pub String);

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The variables a URI matched out of a template, in template order. Values
/// are as they appear in the URI, still percent-encoded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UriVars(Vec<(String, String)>);

impl UriVars {
    /// The value of variable `name`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// All variables, in template order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

impl UriTemplate {
    pub(crate) fn compile(template: &str) -> Result<Self, TemplateError> {
        let bad = |why: &str| TemplateError(format!("template {template:?}: {why}"));
        let mut parts: Vec<Part> = Vec::new();
        let mut rest = template;
        while !rest.is_empty() {
            let Some(open) = rest.find('{') else {
                if rest.contains('}') {
                    return Err(bad("unbalanced '}'"));
                }
                parts.push(Part::Literal(rest.to_owned()));
                break;
            };
            let literal = &rest[..open];
            if literal.contains('}') {
                return Err(bad("unbalanced '}'"));
            }
            if !literal.is_empty() {
                parts.push(Part::Literal(literal.to_owned()));
            }
            let after = &rest[open + 1..];
            let close = after.find('}').ok_or_else(|| bad("unclosed '{'"))?;
            let (reserved, name) = match after[..close].strip_prefix('+') {
                Some(name) => (true, name),
                None => (false, &after[..close]),
            };
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(bad(
                    "only {name} and {+name} are supported, with letters, digits and '_'",
                ));
            }
            if matches!(parts.last(), Some(Part::Var { .. })) {
                return Err(bad(
                    "two variables with nothing between them cannot be told apart",
                ));
            }
            if parts
                .iter()
                .any(|p| matches!(p, Part::Var { name: n, .. } if n == name))
            {
                return Err(bad("a variable appears twice"));
            }
            parts.push(Part::Var {
                name: name.to_owned(),
                reserved,
            });
            rest = &after[close + 1..];
        }
        if parts.is_empty() {
            return Err(bad("empty"));
        }
        Ok(Self { parts })
    }

    /// The variables `uri` matches, or `None` when it does not match.
    pub(crate) fn matches(&self, uri: &str) -> Option<UriVars> {
        let mut vars = Vec::new();
        let mut rest = uri;
        let mut i = 0;
        while i < self.parts.len() {
            match &self.parts[i] {
                Part::Literal(lit) => rest = rest.strip_prefix(lit.as_str())?,
                Part::Var { name, reserved } => {
                    // A variable ends where the next literal begins, or at
                    // the end of the URI.
                    let end = match self.parts.get(i + 1) {
                        Some(Part::Literal(next)) => rest.find(next.as_str())?,
                        _ => rest.len(),
                    };
                    let value = &rest[..end];
                    let segment_only = |c: char| matches!(c, '/' | '?' | '#');
                    if value.is_empty() || (!reserved && value.contains(segment_only)) {
                        return None;
                    }
                    vars.push((name.clone(), value.to_owned()));
                    rest = &rest[end..];
                }
            }
            i += 1;
        }
        rest.is_empty().then_some(UriVars(vars))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn t(s: &str) -> UriTemplate {
        UriTemplate::compile(s).unwrap()
    }

    #[test]
    fn matches_simple_and_reserved_variables() {
        let v = t("file:///{dir}/{name}")
            .matches("file:///src/main.rs")
            .unwrap();
        assert_eq!(
            (v.get("dir"), v.get("name")),
            (Some("src"), Some("main.rs"))
        );
        let v = t("file:///{+path}").matches("file:///a/b/c.txt").unwrap();
        assert_eq!(v.get("path"), Some("a/b/c.txt"));
        let v = t("db://{table}/rows").matches("db://users/rows").unwrap();
        assert_eq!(v.get("table"), Some("users"));
        assert_eq!(v.iter().count(), 1);
        assert!(t("static://thing")
            .matches("static://thing")
            .unwrap()
            .get("x")
            .is_none());
    }

    #[test]
    fn refuses_what_does_not_match() {
        let tpl = t("file:///{dir}/{name}");
        for uri in [
            "file:///src",
            "file:////x",
            "file:///a/b/c",
            "http:///a/b",
            "file:///a/",
            "",
        ] {
            assert!(tpl.matches(uri).is_none(), "{uri:?}");
        }
        assert!(
            t("db://{table}").matches("db://a/b").is_none(),
            "a simple variable stops at '/'"
        );
        assert!(t("q://{x}").matches("q://a?b").is_none());
        assert!(
            t("q://{x}").matches("q://").is_none(),
            "a variable cannot be empty"
        );
        assert!(t("a{x}c").matches("abbc").is_some());
        assert!(t("a{x}c").matches("abbcd").is_none());
    }

    #[test]
    fn refuses_unsupported_templates_at_compile_time() {
        for bad in [
            "", "{", "}", "a}b", "a{b", "{}", "{+}", "{?q}", "{#f}", "{a,b}", "{a*}", "{a}{b}",
            "{a}/{a}", "{a b}", "{/a}",
        ] {
            assert!(UriTemplate::compile(bad).is_err(), "{bad:?}");
        }
    }
}

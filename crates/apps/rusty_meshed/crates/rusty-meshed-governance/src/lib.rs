//! Policy-as-code governance engine -- the Rust port of
//! `meshed.governance.engine` (`GovernanceEngine`, the three built-in
//! policies, and the module-level default-engine constructor).
//!
//! [`GovernanceEngine`] is generic over the payload type: this crate has
//! no dependency on `rusty-meshed-registry`'s concrete `DataProductCreate`
//! type, so any type implementing [`GovernedProduct`] can be evaluated.
//!
//! See `../../capability-manifest.md` (rows GOV-001..009) for the source
//! capabilities this module covers. GOV-003 (wrapping a policy's
//! *unexpected* failure, as opposed to a normal violation) is implemented
//! via [`std::panic::catch_unwind`] around each policy call -- Rust has no
//! exception hierarchy to distinguish "signals a violation" from
//! "misbehaved unexpectedly" the way Python's `except (ValueError,
//! AssertionError)` vs. `except Exception` does, so a policy signals a
//! violation by returning `Err(String)` and an unexpected failure is a
//! genuine Rust panic, caught and wrapped the same way
//! `meshed.governance.engine.GovernanceEngine.evaluate` wraps a
//! non-`ValueError`/`AssertionError` exception. One cosmetic difference
//! from the Python source: a caught panic still prints Rust's default
//! panic message to stderr (installing a custom panic hook to suppress it
//! would be global, process-wide, mutable state -- not safe to toggle
//! per-call in a concurrent server) -- the returned violation string is
//! unaffected either way.
//!
//! GOV-013..020 (the `PortAccessGrant` access-control model, despite the
//! Python module's name `rbac.py` -- see capability-manifest.md's GOV
//! note on scope) live in `rusty-meshed-registry`, since they're a
//! SQLite-backed table + HTTP routes, not part of the governance engine
//! itself.

use std::any::Any;
use std::panic::{self, AssertUnwindSafe};

/// What a governance policy needs to read from a data-product payload.
/// Decouples this crate from `rusty-meshed-registry`'s concrete
/// `DataProductCreate` type -- any type implementing this trait can be
/// evaluated by a [`GovernanceEngine`].
pub trait GovernedProduct {
    /// The data product's description, ported from
    /// `DataProductCreate.description`.
    fn description(&self) -> Option<&str>;
    /// The data product's semantic version string, ported from
    /// `DataProductCreate.version`.
    fn version(&self) -> Option<&str>;
    /// The data product's domain, ported from `DataProductCreate.domain`.
    fn domain(&self) -> Option<&str>;
}

/// What a policy returns: `Ok(())` to pass, `Err(message)` to signal a
/// violation -- the Rust equivalent of a policy function returning `None`
/// vs. raising `ValueError`/`AssertionError` with `message` (GOV-002).
pub type PolicyResult = Result<(), String>;

type PolicyCheck<T> = Box<dyn Fn(&T) -> PolicyResult + Send + Sync>;

/// A single named governance policy. The name mirrors Python's
/// `policy.__name__`, shown in a wrapped `"[POLICY-ERROR]"` message if the
/// policy itself panics (GOV-003).
pub struct Policy<T> {
    name: &'static str,
    check: PolicyCheck<T>,
}

impl<T> Policy<T> {
    /// Builds a named policy from any `Fn(&T) -> PolicyResult`.
    pub fn new(
        name: &'static str,
        check: impl Fn(&T) -> PolicyResult + Send + Sync + 'static,
    ) -> Self {
        Policy {
            name,
            check: Box::new(check),
        }
    }

    fn run(&self, product: &T) -> Result<PolicyResult, Box<dyn Any + Send>> {
        panic::catch_unwind(AssertUnwindSafe(|| (self.check)(product)))
    }
}

/// Evaluates a list of policy callables against a data-product payload --
/// the Rust port of `meshed.governance.engine.GovernanceEngine`.
pub struct GovernanceEngine<T> {
    policies: Vec<Policy<T>>,
}

// Implemented by hand rather than `#[derive(Default)]`: the derive macro
// would add a spurious `T: Default` bound, even though an empty `Vec<Policy<T>>`
// never needs one.
impl<T> Default for GovernanceEngine<T> {
    fn default() -> Self {
        GovernanceEngine::new(Vec::new())
    }
}

impl<T> GovernanceEngine<T> {
    /// Builds an engine from an initial list of policies (GOV-005). Pass
    /// an empty `Vec` for an engine with no policies yet.
    pub fn new(policies: Vec<Policy<T>>) -> Self {
        GovernanceEngine { policies }
    }

    /// Appends a policy, evaluated starting on the *next* [`evaluate`]
    /// call (GOV-004).
    ///
    /// [`evaluate`]: GovernanceEngine::evaluate
    pub fn register_policy(&mut self, policy: Policy<T>) {
        self.policies.push(policy);
    }

    /// Runs every registered policy against `product`, in registration
    /// order, without short-circuiting on the first violation (GOV-001).
    /// Returns one violation message per failing policy; an empty `Vec`
    /// means every policy passed.
    pub fn evaluate(&self, product: &T) -> Vec<String> {
        self.policies
            .iter()
            .filter_map(|policy| match policy.run(product) {
                Ok(outcome) => outcome.err(),
                Err(payload) => Some(format!(
                    "[POLICY-ERROR] {}: {}",
                    policy.name,
                    panic_message(payload.as_ref())
                )),
            })
            .collect()
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        message.to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

/// Enforces that the data product description is at least 20 characters
/// (GOV-006), counting Unicode scalar values the way Python's `len(str)`
/// counts code points.
pub fn require_description_min_length<T: GovernedProduct>(product: &T) -> PolicyResult {
    let description = product.description().unwrap_or("");
    let length = description.chars().count();
    if length < 20 {
        return Err(format!(
            "description must be at least 20 characters (got {length})"
        ));
    }
    Ok(())
}

/// Enforces that the version field is `MAJOR.MINOR.PATCH` (GOV-007).
pub fn require_semantic_version<T: GovernedProduct>(product: &T) -> PolicyResult {
    let version = product.version();
    if !is_semver(version.unwrap_or("")) {
        // Matches the Python source's f-string, which interpolates the
        // raw (possibly-None) value directly rather than the "" fallback
        // used for the match itself -- "got 'None'" when unset, not
        // "got ''".
        let shown = version
            .map(str::to_string)
            .unwrap_or_else(|| "None".to_string());
        return Err(format!(
            "version must be a semantic version (MAJOR.MINOR.PATCH), got '{shown}'"
        ));
    }
    Ok(())
}

fn is_semver(value: &str) -> bool {
    let mut parts = value.split('.');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(major), Some(minor), Some(patch), None) => {
            is_all_digits(major) && is_all_digits(minor) && is_all_digits(patch)
        }
        _ => false,
    }
}

fn is_all_digits(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_digit())
}

/// Enforces that the domain field is entirely lowercase (GOV-008).
pub fn require_domain_lowercase<T: GovernedProduct>(product: &T) -> PolicyResult {
    let domain = product.domain().unwrap_or("");
    let lowered = domain.to_lowercase();
    if domain != lowered {
        return Err(format!("domain must be lowercase, got '{domain}'"));
    }
    Ok(())
}

/// Builds the three-built-in-policy engine that gates data-product
/// creation, matching the Python source's module-level `_DEFAULT_ENGINE`
/// singleton (GOV-009): description-length, then semver, then
/// domain-lowercase, in that fixed order.
pub fn default_engine<T: GovernedProduct + 'static>() -> GovernanceEngine<T> {
    GovernanceEngine::new(vec![
        Policy::new(
            "require_description_min_length",
            require_description_min_length::<T>,
        ),
        Policy::new("require_semantic_version", require_semantic_version::<T>),
        Policy::new("require_domain_lowercase", require_domain_lowercase::<T>),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestProduct {
        description: Option<String>,
        version: Option<String>,
        domain: Option<String>,
    }

    impl TestProduct {
        fn valid() -> Self {
            TestProduct {
                description: Some("A well-described data product.".to_string()),
                version: Some("1.0.0".to_string()),
                domain: Some("manpower".to_string()),
            }
        }
    }

    impl GovernedProduct for TestProduct {
        fn description(&self) -> Option<&str> {
            self.description.as_deref()
        }
        fn version(&self) -> Option<&str> {
            self.version.as_deref()
        }
        fn domain(&self) -> Option<&str> {
            self.domain.as_deref()
        }
    }

    #[test]
    fn engine_no_policies_returns_empty_violations() {
        let engine: GovernanceEngine<TestProduct> = GovernanceEngine::default();
        assert_eq!(engine.evaluate(&TestProduct::valid()), Vec::<String>::new());
    }

    #[test]
    fn engine_single_passing_policy_returns_empty_violations() {
        let engine =
            GovernanceEngine::new(vec![Policy::new("always_pass", |_: &TestProduct| Ok(()))]);
        assert_eq!(engine.evaluate(&TestProduct::valid()), Vec::<String>::new());
    }

    #[test]
    fn engine_single_failing_policy_returns_one_violation() {
        let engine = GovernanceEngine::new(vec![Policy::new("always_fail", |_: &TestProduct| {
            Err("description too short".to_string())
        })]);
        let violations = engine.evaluate(&TestProduct::valid());
        assert_eq!(violations, vec!["description too short".to_string()]);
    }

    #[test]
    fn engine_wraps_a_panicking_policy_with_policy_error_prefix() {
        let engine = GovernanceEngine::new(vec![Policy::new("blow_up", |_: &TestProduct| {
            panic!("unexpected internal failure")
        })]);
        let violations = engine.evaluate(&TestProduct::valid());
        assert_eq!(violations.len(), 1);
        assert!(violations[0].starts_with("[POLICY-ERROR]"));
        assert!(violations[0].contains("blow_up"));
        assert!(violations[0].contains("unexpected internal failure"));
    }

    #[test]
    fn engine_mixed_policies_returns_only_failing_violations() {
        let engine = GovernanceEngine::new(vec![
            Policy::new("pass_policy", |_: &TestProduct| Ok(())),
            Policy::new("fail_policy", |_: &TestProduct| {
                Err("this one fails".to_string())
            }),
            Policy::new("pass_policy_2", |_: &TestProduct| Ok(())),
        ]);
        let violations = engine.evaluate(&TestProduct::valid());
        assert_eq!(violations, vec!["this one fails".to_string()]);
    }

    #[test]
    fn register_policy_appends_callable_evaluated_on_next_call() {
        let mut engine: GovernanceEngine<TestProduct> = GovernanceEngine::default();
        assert_eq!(engine.evaluate(&TestProduct::valid()), Vec::<String>::new());

        engine.register_policy(Policy::new("registered", |_: &TestProduct| {
            Err("registered policy violated".to_string())
        }));
        let violations = engine.evaluate(&TestProduct::valid());
        assert_eq!(violations, vec!["registered policy violated".to_string()]);
    }

    #[test]
    fn require_description_min_length_rejects_short_description() {
        let product = TestProduct {
            description: Some("Too short".to_string()),
            ..TestProduct::valid()
        };
        let err = require_description_min_length(&product).unwrap_err();
        assert_eq!(err, "description must be at least 20 characters (got 9)");
    }

    #[test]
    fn require_description_min_length_accepts_long_description() {
        let product = TestProduct {
            description: Some("A".repeat(20)),
            ..TestProduct::valid()
        };
        assert!(require_description_min_length(&product).is_ok());
    }

    #[test]
    fn require_description_min_length_treats_missing_description_as_empty() {
        let product = TestProduct {
            description: None,
            ..TestProduct::valid()
        };
        let err = require_description_min_length(&product).unwrap_err();
        assert_eq!(err, "description must be at least 20 characters (got 0)");
    }

    #[test]
    fn require_semantic_version_rejects_incomplete_version() {
        let product = TestProduct {
            version: Some("v1.0".to_string()),
            ..TestProduct::valid()
        };
        let err = require_semantic_version(&product).unwrap_err();
        assert_eq!(
            err,
            "version must be a semantic version (MAJOR.MINOR.PATCH), got 'v1.0'"
        );
    }

    #[test]
    fn require_semantic_version_accepts_valid_semver() {
        let product = TestProduct {
            version: Some("1.0.0".to_string()),
            ..TestProduct::valid()
        };
        assert!(require_semantic_version(&product).is_ok());
    }

    #[test]
    fn require_semantic_version_rejects_prerelease_suffix() {
        let product = TestProduct {
            version: Some("1.0.0-beta".to_string()),
            ..TestProduct::valid()
        };
        assert!(require_semantic_version(&product).is_err());
    }

    #[test]
    fn require_semantic_version_reports_none_literal_for_missing_version() {
        let product = TestProduct {
            version: None,
            ..TestProduct::valid()
        };
        let err = require_semantic_version(&product).unwrap_err();
        assert_eq!(
            err,
            "version must be a semantic version (MAJOR.MINOR.PATCH), got 'None'"
        );
    }

    #[test]
    fn require_domain_lowercase_rejects_mixed_case_domain() {
        let product = TestProduct {
            domain: Some("Manpower".to_string()),
            ..TestProduct::valid()
        };
        let err = require_domain_lowercase(&product).unwrap_err();
        assert_eq!(err, "domain must be lowercase, got 'Manpower'");
    }

    #[test]
    fn require_domain_lowercase_accepts_lowercase_domain() {
        let product = TestProduct {
            domain: Some("manpower".to_string()),
            ..TestProduct::valid()
        };
        assert!(require_domain_lowercase(&product).is_ok());
    }

    #[test]
    fn default_engine_runs_all_three_built_in_policies_in_order() {
        let engine = default_engine::<TestProduct>();
        let product = TestProduct {
            description: Some("short".to_string()),
            version: Some("bad".to_string()),
            domain: Some("BAD".to_string()),
        };
        let violations = engine.evaluate(&product);
        assert_eq!(violations.len(), 3);
        assert!(violations[0].starts_with("description must be"));
        assert!(violations[1].starts_with("version must be"));
        assert!(violations[2].starts_with("domain must be"));
    }

    #[test]
    fn default_engine_passes_a_fully_valid_product() {
        let engine = default_engine::<TestProduct>();
        assert_eq!(engine.evaluate(&TestProduct::valid()), Vec::<String>::new());
    }
}

//! Certification path building and validation — stage 2b-ii, RFC 5280 §6.1.
//!
//! This is the module that answers the question the whole issue is about:
//! given a certificate a peer presented, some intermediates it offered, and a
//! set of trust anchors, does a valid chain exist from that certificate to one
//! of those anchors?
//!
//! Everything below it was building toward this. [`super::x509`] says what
//! certificates claim; [`super::verify`] says who signed what. Neither can be
//! wrong in a way that matters on its own. This one can.
//!
//! # What a success means, precisely
//!
//! [`validate_path`] returning [`VerifiedPath`] means every one of these held:
//!
//! - each certificate's `issuer` is byte-identical to the next one's `subject`
//!   (RFC 5280 §6.1.3(a)(4)),
//! - each certificate's signature verifies under the next one's public key,
//!   terminating at a trust anchor's key,
//! - every certificate was within its validity period at the supplied time,
//! - every CA in the path asserts `basicConstraints` with `cA` true,
//! - every CA that carries `keyUsage` asserts `keyCertSign`,
//! - no `pathLenConstraint` in the path was exceeded,
//! - no certificate carried a critical extension this implementation does not
//!   understand,
//! - the end-entity certificate permits the required extended key usage.
//!
//! # What it still does not mean
//!
//! **Not that the certificate is valid for any particular name.** Hostname
//! and IP matching is stage 2b-iii and does not exist. A chain that validates
//! here is a chain to a trusted CA; it says nothing about *who* the peer is.
//! Using this alone to accept a TLS connection would accept any certificate
//! from any public CA for any site — which is a real and well-understood
//! attack, not a theoretical gap.
//!
//! # Name constraints, and why not implementing them is safe here
//!
//! RFC 5280 §4.2.1.10 name constraints are not implemented. That would be a
//! serious hole in most designs, because a name-constrained intermediate is
//! constrained precisely so it *cannot* issue for names outside its subtree,
//! and ignoring the constraint hands it the whole namespace.
//!
//! It is not a hole here, and the reason is structural rather than lucky:
//! `nameConstraints` MUST be marked critical (§4.2.1.10), [`super::x509`]
//! reports every critical extension it does not understand, and this module
//! **rejects** any certificate with one. So a name-constrained intermediate
//! does not get its constraint ignored — the whole chain is refused.
//!
//! That is the correct failure direction, and it has a real cost: chains
//! through name-constrained intermediates, which genuinely exist, are refused
//! rather than validated. That is a capability gap, and it is exactly the sort
//! of thing that gets "fixed" later by relaxing the critical-extension check.
//! It must not be. The fix is to implement name constraints.
//!
//! # Path building is a search, and searches are attackable
//!
//! An attacker supplies the intermediates. A naive builder that explores
//! every ordering of them does exponential work on a small input — a real
//! denial of service that has hit real libraries. Two bounds apply here:
//! [`PathOptions::max_path_length`] caps depth, and
//! [`PathOptions::max_signature_checks`] caps total signature verifications
//! across the entire search. Both are hard limits that end the search rather
//! than merely discouraging it, and no certificate can raise either.

use super::verify::{verify_signature, VerifyError};
use super::x509::{oid, Certificate, SubjectPublicKeyInfo};
use crate::handrolled::der::ObjectIdentifier;

/// A trust anchor: a name and a public key that are trusted a priori.
///
/// RFC 5280 §6.1 validates a path *to* an anchor, never the anchor itself.
/// That is why this is a name and a key rather than a certificate — the
/// certificate a store hands out is a convenient container for those two
/// things, and its signature, validity period, and extensions are not
/// consulted. A trust anchor is trusted because it is in the store, not
/// because of anything it says about itself.
#[derive(Clone, Copy, Debug)]
pub struct TrustAnchor<'a> {
    /// The anchor's encoded `subject` name, matched against a certificate's
    /// encoded `issuer`.
    pub subject: &'a [u8],
    /// The anchor's public key, which terminates the chain of signatures.
    pub public_key: SubjectPublicKeyInfo<'a>,
}

impl<'a> TrustAnchor<'a> {
    /// Take the name and key out of a certificate from a trust store.
    ///
    /// Nothing else in the certificate is read, deliberately — see the type's
    /// documentation. In particular an expired root is still a usable anchor,
    /// which is not an oversight: §6.1 never checks the anchor's validity
    /// period, and stores routinely carry roots whose self-signed certificate
    /// has expired while the key remains trusted.
    pub fn from_certificate(certificate: &Certificate<'a>) -> Self {
        Self {
            subject: certificate.subject(),
            public_key: certificate.subject_public_key_info(),
        }
    }
}

/// Knobs for [`validate_path`], all of them limits rather than relaxations.
#[derive(Clone, Copy, Debug)]
pub struct PathOptions {
    /// The moment to validate against, in seconds since the Unix epoch.
    ///
    /// Supplied rather than read from a clock: this module has no business
    /// deciding what "now" means, and a caller that wants to validate against
    /// a past moment (auditing a captured handshake) should not have to lie
    /// to a system call to do it.
    pub time: i64,
    /// The most certificates a path may contain, end-entity and intermediates
    /// together, excluding the anchor.
    ///
    /// A bound on attacker-controlled search depth, not a protocol rule.
    pub max_path_length: usize,
    /// The most signature verifications the whole search may perform.
    ///
    /// The real defence against a pathological intermediate set: depth alone
    /// does not bound the *number* of candidate paths, and signature
    /// verification is the expensive step.
    pub max_signature_checks: usize,
    /// An extended key usage the end-entity certificate must permit.
    ///
    /// `None` skips the check. A certificate with no `extendedKeyUsage`
    /// extension permits everything, so absence of the extension satisfies
    /// any requirement — that asymmetry is the extension's actual semantics
    /// and getting it backwards would refuse most of the web.
    pub required_eku: Option<ObjectIdentifier<'static>>,
}

impl Default for PathOptions {
    /// Server authentication, a depth of eight, and a budget of a hundred
    /// signature checks.
    ///
    /// Eight is comfortably more than any real chain (the web PKI runs three
    /// or four) and far below anything that costs real time. `time` defaults
    /// to zero, which is 1970 and will fail every certificate — a caller must
    /// supply it deliberately rather than get a plausible-looking default that
    /// silently validates against the wrong moment.
    fn default() -> Self {
        Self {
            time: 0,
            max_path_length: 8,
            max_signature_checks: 100,
            required_eku: Some(oid::EKU_SERVER_AUTH),
        }
    }
}

/// A path that validated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedPath {
    /// Index into the `anchors` slice of the anchor the path terminated at.
    pub anchor: usize,
    /// Indices into the `intermediates` slice, ordered from the end-entity
    /// certificate upward.
    ///
    /// Empty when the end-entity certificate was issued directly by an anchor.
    pub intermediates: Vec<usize>,
}

/// Why no valid path was found.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PathError {
    /// No chain of names and signatures reached a trust anchor.
    ///
    /// The catch-all, reported when nothing more specific was learned. Other
    /// variants are preferred where the search got far enough to know one.
    NoPathToTrustAnchor,
    /// A certificate in the path was not yet valid at the supplied time.
    NotYetValid {
        /// The `notBefore` that had not arrived.
        not_before: i64,
    },
    /// A certificate in the path had expired at the supplied time.
    Expired {
        /// The `notAfter` that had passed.
        not_after: i64,
    },
    /// A certificate used as a CA did not assert `basicConstraints` with `cA`
    /// true.
    ///
    /// Including the case where the extension is absent entirely: RFC 5280
    /// §6.1.4(k) requires it to be *present and true*, and treating absence as
    /// permission is the Basic Constraints bug that let any leaf certificate
    /// sign for any site.
    NotACertificateAuthority,
    /// A CA carried `keyUsage` without `keyCertSign`.
    MissingKeyCertSign,
    /// A `pathLenConstraint` in the path was exceeded.
    PathLengthExceeded,
    /// A certificate carried a critical extension this implementation does not
    /// understand, so it cannot be validated safely.
    ///
    /// Notably includes `nameConstraints` — see the module docs.
    UnhandledCriticalExtension(Vec<u8>),
    /// The end-entity certificate did not permit the required extended key
    /// usage.
    RequiredEkuMissing,
    /// A certificate's `extendedKeyUsage` extension was malformed.
    MalformedExtendedKeyUsage,
    /// The search hit [`PathOptions::max_signature_checks`].
    ///
    /// Not "no path exists" — the search was stopped before it could say.
    SearchBudgetExhausted,
    /// A signature in an otherwise plausible path did not verify, or used an
    /// algorithm this implementation refuses.
    Signature(VerifyError),
}

impl core::fmt::Display for PathError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoPathToTrustAnchor => f.write_str("no path to a trust anchor"),
            Self::NotYetValid { not_before } => {
                write!(f, "a certificate is not valid until {not_before}")
            }
            Self::Expired { not_after } => write!(f, "a certificate expired at {not_after}"),
            Self::NotACertificateAuthority => {
                f.write_str("a certificate used as a CA does not assert basicConstraints cA")
            }
            Self::MissingKeyCertSign => {
                f.write_str("a CA's keyUsage does not permit signing certificates")
            }
            Self::PathLengthExceeded => f.write_str("a pathLenConstraint was exceeded"),
            Self::UnhandledCriticalExtension(oid) => {
                write!(f, "unhandled critical extension {oid:02x?}")
            }
            Self::RequiredEkuMissing => {
                f.write_str("the end-entity certificate does not permit the required key usage")
            }
            Self::MalformedExtendedKeyUsage => f.write_str("malformed extendedKeyUsage"),
            Self::SearchBudgetExhausted => {
                f.write_str("path search budget exhausted before a path was found")
            }
            Self::Signature(err) => write!(f, "signature: {err}"),
        }
    }
}

impl std::error::Error for PathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Signature(err) => Some(err),
            _ => None,
        }
    }
}

impl From<VerifyError> for PathError {
    fn from(err: VerifyError) -> Self {
        Self::Signature(err)
    }
}

/// How specific an error is, for choosing which one to report.
///
/// A search tries many branches and most fail uninterestingly. Reporting the
/// most specific thing learned is what makes the difference between "no path
/// to a trust anchor" and "your intermediate expired last Tuesday".
fn specificity(error: &PathError) -> u8 {
    match error {
        PathError::NoPathToTrustAnchor => 0,
        PathError::Signature(VerifyError::BadSignature) => 1,
        _ => 2,
    }
}

/// Build and validate a certification path.
///
/// `intermediates` are candidates, not an ordered chain: a peer sends them in
/// whatever order it likes, may send irrelevant ones, and may omit needed
/// ones. The search finds an order that works, or reports that none does.
///
/// Read the module docs before using this to make a trust decision — in
/// particular the part about name matching, which this does not do.
pub fn validate_path(
    end_entity: &Certificate<'_>,
    intermediates: &[Certificate<'_>],
    anchors: &[TrustAnchor<'_>],
    options: &PathOptions,
) -> Result<VerifiedPath, PathError> {
    // The end-entity certificate's own checks, which do not depend on the
    // path and so are done once rather than per candidate branch.
    check_validity(end_entity, options.time)?;
    check_no_unhandled_critical(end_entity)?;
    check_eku(end_entity, options.required_eku)?;

    if options.max_path_length == 0 {
        return Err(PathError::NoPathToTrustAnchor);
    }

    let mut budget = options.max_signature_checks;
    let mut used = Vec::new();
    let mut best = PathError::NoPathToTrustAnchor;

    match search(
        end_entity,
        intermediates,
        anchors,
        options,
        1,
        &mut used,
        &mut budget,
        &mut best,
    ) {
        Some(anchor) => Ok(VerifiedPath {
            anchor,
            intermediates: used,
        }),
        None => Err(best),
    }
}

/// Depth-first search for an issuer of `current`, returning the anchor index.
///
/// Recursion depth is bounded by `options.max_path_length`, which is this
/// crate's constant and not anything a certificate can influence, so the
/// stack is bounded regardless of input.
#[allow(clippy::too_many_arguments)]
fn search(
    current: &Certificate<'_>,
    intermediates: &[Certificate<'_>],
    anchors: &[TrustAnchor<'_>],
    options: &PathOptions,
    depth: usize,
    used: &mut Vec<usize>,
    budget: &mut usize,
    best: &mut PathError,
) -> Option<usize> {
    // Anchors first: the shortest path is preferred, and a chain that can end
    // here should not spend budget exploring longer ones.
    for (index, anchor) in anchors.iter().enumerate() {
        if anchor.subject != current.issuer() {
            continue;
        }
        if *budget == 0 {
            record(best, PathError::SearchBudgetExhausted);
            return None;
        }
        *budget -= 1;
        match verify_signature(current, &anchor.public_key) {
            Ok(()) => {
                if let Err(err) = check_path_length(intermediates, used) {
                    record(best, err);
                    continue;
                }
                return Some(index);
            }
            Err(err) => record(best, PathError::Signature(err)),
        }
    }

    if depth >= options.max_path_length {
        return None;
    }

    for (index, candidate) in intermediates.iter().enumerate() {
        if used.contains(&index) {
            // Reusing a certificate would be a cycle, and a cycle is how a
            // path search turns into a hang.
            continue;
        }
        if candidate.subject() != current.issuer() {
            continue;
        }

        // Everything about the candidate that does not depend on what is
        // above it, checked before spending a signature verification on it.
        if let Err(err) = check_ca(candidate, options.time) {
            record(best, err);
            continue;
        }

        if *budget == 0 {
            record(best, PathError::SearchBudgetExhausted);
            return None;
        }
        *budget -= 1;
        if let Err(err) = verify_signature(current, &candidate.subject_public_key_info()) {
            record(best, PathError::Signature(err));
            continue;
        }

        used.push(index);
        if let Some(anchor) = search(
            candidate,
            intermediates,
            anchors,
            options,
            depth + 1,
            used,
            budget,
            best,
        ) {
            return Some(anchor);
        }
        used.pop();
    }

    None
}

fn record(best: &mut PathError, candidate: PathError) {
    if specificity(&candidate) > specificity(best) {
        *best = candidate;
    }
}

/// Everything a certificate must satisfy to act as a CA in a path.
fn check_ca(certificate: &Certificate<'_>, time: i64) -> Result<(), PathError> {
    check_validity(certificate, time)?;
    check_no_unhandled_critical(certificate)?;

    // RFC 5280 §6.1.4(k): basicConstraints must be present with cA true.
    // Absence is not permission — treating it as permission is the bug that
    // let a leaf certificate sign for any site.
    match certificate.extensions().basic_constraints() {
        Some(constraints) if constraints.is_ca => {}
        _ => return Err(PathError::NotACertificateAuthority),
    }

    // §6.1.4(n): if keyUsage is present, keyCertSign must be set. Absent
    // keyUsage means unrestricted, which is the extension's semantics.
    if let Some(usage) = certificate.extensions().key_usage() {
        if !usage.key_cert_sign() {
            return Err(PathError::MissingKeyCertSign);
        }
    }

    Ok(())
}

fn check_validity(certificate: &Certificate<'_>, time: i64) -> Result<(), PathError> {
    let validity = certificate.validity();
    if time < validity.not_before {
        return Err(PathError::NotYetValid {
            not_before: validity.not_before,
        });
    }
    if time > validity.not_after {
        return Err(PathError::Expired {
            not_after: validity.not_after,
        });
    }
    Ok(())
}

/// RFC 5280 §6.1.3(f): a certificate carrying a critical extension the
/// validator does not recognise cannot be processed, and must be rejected.
///
/// This is what makes unimplemented name constraints fail closed — see the
/// module docs.
fn check_no_unhandled_critical(certificate: &Certificate<'_>) -> Result<(), PathError> {
    match certificate.extensions().unhandled_critical().first() {
        Some(oid) => Err(PathError::UnhandledCriticalExtension(
            oid.as_bytes().to_vec(),
        )),
        None => Ok(()),
    }
}

/// The end-entity certificate must permit `required`, if one is required.
///
/// An absent `extendedKeyUsage` extension permits every purpose; a present one
/// permits only what it lists, plus `anyExtendedKeyUsage`. Inverting that
/// would refuse most of the web, and ignoring it would let a
/// code-signing-only certificate authenticate a server.
fn check_eku(
    certificate: &Certificate<'_>,
    required: Option<ObjectIdentifier<'static>>,
) -> Result<(), PathError> {
    let Some(required) = required else {
        return Ok(());
    };
    if !certificate.extensions().has_extended_key_usage() {
        return Ok(());
    }

    for purpose in certificate.extensions().extended_key_usage() {
        let purpose = purpose.map_err(|_| PathError::MalformedExtendedKeyUsage)?;
        if purpose == required || purpose == oid::EKU_ANY {
            return Ok(());
        }
    }
    Err(PathError::RequiredEkuMissing)
}

/// RFC 5280 §6.1.4(l) and (m), applied to an assembled path.
///
/// Processing runs from the anchor down, which is the direction the RFC
/// describes and the only one in which the running limit makes sense:
/// `max_path_length` starts unbounded, each non-self-issued CA consumes one,
/// and any `pathLenConstraint` tightens it for everything below.
fn check_path_length(intermediates: &[Certificate<'_>], used: &[usize]) -> Result<(), PathError> {
    let mut remaining = usize::MAX;

    // `used` runs from the end-entity certificate upward, so walking it in
    // reverse is walking down from the anchor.
    for &index in used.iter().rev() {
        let certificate = &intermediates[index];

        // §6.1.4(l): self-issued certificates do not consume path length —
        // they are a CA re-issuing to itself (a key rollover), not a step
        // further from the anchor.
        if !certificate.is_self_issued() {
            if remaining == 0 {
                return Err(PathError::PathLengthExceeded);
            }
            remaining -= 1;
        }

        // §6.1.4(m).
        if let Some(constraint) = certificate
            .extensions()
            .basic_constraints()
            .and_then(|c| c.path_len_constraint)
        {
            let constraint = usize::try_from(constraint).unwrap_or(usize::MAX);
            remaining = remaining.min(constraint);
        }
    }

    Ok(())
}

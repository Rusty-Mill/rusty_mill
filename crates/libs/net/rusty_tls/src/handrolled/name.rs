//! Server name matching and name constraints — stage 2b-iii.
//!
//! Two questions, both about names, both answered here because they share the
//! same label arithmetic:
//!
//! - **Is this certificate valid for the server I meant to reach?**
//!   ([`verify_server_name`])
//! - **Was the CA that issued it permitted to issue for that name?**
//!   ([`check_name_constraints`], driven from [`super::path`])
//!
//! Together with [`super::path`] these complete the trust decision. Before
//! this stage, a validated chain said "some trusted CA issued this" and
//! nothing about *who* the certificate was for — which on its own would accept
//! any certificate from any public CA for any site.
//!
//! # There is no Common Name fallback, deliberately
//!
//! A certificate's `subject` Common Name is **never** consulted as a server
//! name here. RFC 2818's CN fallback was obsoleted by RFC 6125 and removed
//! from browsers, and it is the mechanism behind a long run of real attacks:
//! a CN is free text that no CA validates as a domain, so treating it as one
//! turns "this CA vouched for a string" into "this CA vouched for a host".
//!
//! A certificate with no `subjectAltName` therefore matches nothing at all.
//! That is the intended outcome and not a gap to fill in later.
//!
//! The same decision applies to name *constraints*: since a CN is not a DNS
//! name for matching, it is not one for constraining either. That combination
//! is coherent rather than convenient — a certificate whose only name-like
//! field is a CN cannot authenticate anything, so there is nothing for a
//! constraint to protect.
//!
//! # Wildcards, and the parts of them that are refused
//!
//! `*.example.com` matches exactly one label in that position:
//! `www.example.com` yes, `example.com` no, `a.b.example.com` no. Beyond RFC
//! 6125, three things are refused outright:
//!
//! - **Partial wildcards** (`www*.example.com`). RFC 6125 permits them; the
//!   CA/Browser Forum forbids them and no browser accepts them.
//! - **A wildcard anywhere but the leftmost label** (`a.*.example.com`).
//! - **A wildcard with fewer than two labels behind it** (`*.com`), which
//!   would be a certificate for an entire top-level domain.
//!
//! The last is a policy judgment rather than a rule from the RFC, and it is a
//! crude stand-in for a public suffix list, which this crate does not carry.
//! It stops the obvious case and does not pretend to stop the subtle one
//! (`*.co.uk` is still accepted). Named here so nobody mistakes it for
//! completeness.
//!
//! # Embedded NULs
//!
//! A `dNSName` containing a NUL is malformed and is skipped. This is the
//! null-prefix attack (CVE-2009-2408), where `evil.com\0good.com` reads as
//! `good.com` to anything using C string semantics.
//!
//! [`super::x509`] preserves the NUL rather than dropping or rejecting it, and
//! that decision was made *for this module*: a parser that silently trimmed at
//! the NUL would hand this code `evil.com` and a parser that rejected the
//! certificate outright would hide a detectable attack behind a generic parse
//! error. Preserved, it is visible and refused here.

use std::net::IpAddr;

use super::der::{Reader, Tag};
use super::x509::{Certificate, GeneralName};

/// The identity a caller is trying to reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerName<'a> {
    /// A DNS hostname, as the caller wrote it.
    Dns(&'a str),
    /// An IP address literal.
    ///
    /// Separate from [`ServerName::Dns`] because they are matched against
    /// different SAN types and never against each other: a certificate for
    /// the *string* "192.0.2.1" does not authenticate the *address*
    /// 192.0.2.1.
    Ip(IpAddr),
}

/// Why a name did not match, or a constraint was not satisfied.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NameError {
    /// No `subjectAltName` entry matched the requested name.
    ///
    /// Includes the case of a certificate with no `subjectAltName` at all,
    /// which matches nothing — see the module docs on the absent CN fallback.
    NoMatchingSubjectAltName,
    /// The name the caller asked for is not a syntactically valid DNS name.
    ///
    /// A caller error rather than a certificate problem, and worth
    /// distinguishing: it means the question was malformed, not that the
    /// answer was no.
    MalformedReferenceName,
    /// A name in the certificate fell inside an excluded subtree.
    ExcludedByNameConstraint,
    /// A name in the certificate fell outside every permitted subtree of its
    /// type.
    NotPermittedByNameConstraint,
    /// A `NameConstraints` extension used a general-name type this
    /// implementation cannot evaluate.
    ///
    /// Refused rather than ignored: a constraint that is not enforced is a
    /// constraint that does not exist, and the CA wrote it down because it
    /// meant to be limited.
    UnsupportedNameConstraint {
        /// The context-specific tag of the general-name type.
        tag: u8,
    },
    /// A `NameConstraints` extension was structurally invalid, or used the
    /// `minimum`/`maximum` fields RFC 5280 §4.2.1.10 forbids.
    MalformedNameConstraints,
}

impl core::fmt::Display for NameError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoMatchingSubjectAltName => {
                f.write_str("no subjectAltName entry matches the requested name")
            }
            Self::MalformedReferenceName => {
                f.write_str("the requested name is not a valid DNS name")
            }
            Self::ExcludedByNameConstraint => {
                f.write_str("a name falls inside an excluded subtree")
            }
            Self::NotPermittedByNameConstraint => {
                f.write_str("a name falls outside every permitted subtree")
            }
            Self::UnsupportedNameConstraint { tag } => write!(
                f,
                "a name constraint uses general-name type {tag}, which cannot be enforced"
            ),
            Self::MalformedNameConstraints => f.write_str("malformed nameConstraints"),
        }
    }
}

impl std::error::Error for NameError {}

// ---------------------------------------------------------------------------
// DNS name syntax
// ---------------------------------------------------------------------------

/// Longest permitted presentation-format DNS name, per RFC 1035 §2.3.4.
const MAX_NAME_LEN: usize = 253;
/// Longest permitted label, per RFC 1035 §2.3.4.
const MAX_LABEL_LEN: usize = 63;

/// Is this a syntactically usable DNS name?
///
/// Strict on purpose. Every relaxation here is a way for two names that look
/// different to compare equal, and comparing equal is the whole question.
/// Rejected: empty names and labels, anything over the length limits, a
/// trailing dot, non-ASCII, and any byte outside letters, digits, hyphen, and
/// the dots that separate labels — which excludes NUL without needing a
/// special case for it.
fn is_valid_dns_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_NAME_LEN || !name.is_ascii() {
        return false;
    }
    // A trailing dot is the root label. Some callers write it; accepting it
    // would mean "example.com" and "example.com." are two spellings of one
    // name, so it is refused rather than normalised away.
    if name.ends_with('.') || name.starts_with('.') {
        return false;
    }

    name.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= MAX_LABEL_LEN
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}

/// Case-insensitive ASCII equality. DNS is case-insensitive; nothing here
/// touches non-ASCII, because certificates carry punycode A-labels and
/// comparing those is a byte comparison.
fn eq_ignore_case(left: &str, right: &str) -> bool {
    left.len() == right.len() && left.eq_ignore_ascii_case(right)
}

/// Does a presented `dNSName` match a reference name?
///
/// `presented` may carry a leftmost wildcard label; see the module docs for
/// the three wildcard forms that are refused.
fn dns_name_matches(presented: &str, reference: &str) -> bool {
    match presented.strip_prefix("*.") {
        None => is_valid_dns_name(presented) && eq_ignore_case(presented, reference),
        Some(suffix) => {
            // The remainder must itself be a valid name, and must have at
            // least two labels: `*.com` is a certificate for a whole TLD.
            if !is_valid_dns_name(suffix) || !suffix.contains('.') {
                return false;
            }
            // The wildcard replaces exactly one label, so the reference must
            // have a first label and the rest must match the suffix exactly.
            match reference.split_once('.') {
                Some((first, rest)) => !first.is_empty() && eq_ignore_case(rest, suffix),
                None => false,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Server name matching
// ---------------------------------------------------------------------------

/// Does this certificate authenticate `name`?
///
/// Only `subjectAltName` is consulted — never the Common Name. A certificate
/// with no `subjectAltName` matches nothing.
///
/// Malformed SAN entries are skipped rather than fatal: one unparseable name
/// should not stop a different, valid one from matching. What is *not* skipped
/// is a malformed reference name, which is refused outright, because a
/// caller asking about a nonsense name should learn that rather than get a
/// plain "no".
pub fn verify_server_name(
    certificate: &Certificate<'_>,
    name: &ServerName<'_>,
) -> Result<(), NameError> {
    match name {
        ServerName::Dns(reference) => {
            if !is_valid_dns_name(reference) {
                return Err(NameError::MalformedReferenceName);
            }
            for entry in certificate.extensions().subject_alt_names() {
                if let Ok(GeneralName::DnsName(presented)) = entry {
                    if dns_name_matches(presented, reference) {
                        return Ok(());
                    }
                }
            }
        }
        ServerName::Ip(address) => {
            let wanted = ip_octets(*address);
            for entry in certificate.extensions().subject_alt_names() {
                if let Ok(GeneralName::IpAddress(presented)) = entry {
                    if presented == wanted.as_slice() {
                        return Ok(());
                    }
                }
            }
        }
    }
    Err(NameError::NoMatchingSubjectAltName)
}

fn ip_octets(address: IpAddr) -> Vec<u8> {
    match address {
        IpAddr::V4(v4) => v4.octets().to_vec(),
        IpAddr::V6(v6) => v6.octets().to_vec(),
    }
}

// ---------------------------------------------------------------------------
// Name constraints
// ---------------------------------------------------------------------------

/// One `GeneralSubtree` base this implementation can evaluate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Subtree<'a> {
    /// A `dNSName` base. An empty base matches every DNS name.
    Dns(&'a str),
    /// An `iPAddress` base: address followed by mask, 8 octets for IPv4 and
    /// 32 for IPv6.
    Ip { address: &'a [u8], mask: &'a [u8] },
}

/// The parsed permitted and excluded subtrees of one CA's constraints.
#[derive(Debug, Default)]
struct Constraints<'a> {
    permitted: Vec<Subtree<'a>>,
    excluded: Vec<Subtree<'a>>,
}

/// Parse a `NameConstraints` extension body.
///
/// Any general-name type that cannot be evaluated is an error rather than a
/// skipped entry. That is the whole safety property: a constraint this code
/// ignores is a constraint the CA does not have.
fn parse_constraints(body: &[u8]) -> Result<Constraints<'_>, NameError> {
    let mut reader = Reader::new(body);
    let mut constraints = Constraints::default();

    for (tag, target) in [
        (Tag::context(0, true), &mut constraints.permitted),
        (Tag::context(1, true), &mut constraints.excluded),
    ] {
        let Some(value) = reader
            .read_optional(tag)
            .map_err(|_| NameError::MalformedNameConstraints)?
        else {
            continue;
        };
        let mut subtrees = Reader::new(value.contents);
        // RFC 5280 §4.2.1.10: GeneralSubtrees has at least one entry.
        if subtrees.is_empty() {
            return Err(NameError::MalformedNameConstraints);
        }
        while !subtrees.is_empty() {
            target.push(parse_subtree(&mut subtrees)?);
        }
    }

    reader
        .finish()
        .map_err(|_| NameError::MalformedNameConstraints)?;
    Ok(constraints)
}

fn parse_subtree<'a>(reader: &mut Reader<'a>) -> Result<Subtree<'a>, NameError> {
    let sequence = reader
        .read(Tag::SEQUENCE)
        .map_err(|_| NameError::MalformedNameConstraints)?;
    let mut inner = Reader::new(sequence.contents);
    let base = inner
        .read_any()
        .map_err(|_| NameError::MalformedNameConstraints)?;

    // RFC 5280 §4.2.1.10: "minimum MUST be zero, and maximum MUST be absent".
    // Since DER omits DEFAULT values, a conforming subtree has neither field,
    // so anything left over is out of profile.
    if !inner.is_empty() {
        return Err(NameError::MalformedNameConstraints);
    }

    match base.tag.0 {
        // dNSName [2] IMPLICIT IA5String
        0x82 => {
            let text = core::str::from_utf8(base.contents)
                .map_err(|_| NameError::MalformedNameConstraints)?;
            if !text.is_ascii() {
                return Err(NameError::MalformedNameConstraints);
            }
            Ok(Subtree::Dns(text))
        }
        // iPAddress [7] IMPLICIT OCTET STRING — address and mask together.
        0x87 => match base.contents.len() {
            8 => Ok(Subtree::Ip {
                address: &base.contents[..4],
                mask: &base.contents[4..],
            }),
            32 => Ok(Subtree::Ip {
                address: &base.contents[..16],
                mask: &base.contents[16..],
            }),
            _ => Err(NameError::MalformedNameConstraints),
        },
        // Everything else — directoryName, rfc822Name, URI, otherName — is
        // refused rather than ignored. Some real CAs use directoryName
        // constraints, so this is a genuine capability gap, and it is the
        // safe side of one.
        tag => Err(NameError::UnsupportedNameConstraint { tag: tag & 0x1f }),
    }
}

/// Does `name` fall inside `subtree`?
fn dns_within(subtree: &str, name: &str) -> bool {
    // RFC 5280 §4.2.1.10: an empty base matches everything of that type.
    if subtree.is_empty() {
        return true;
    }
    if eq_ignore_case(subtree, name) {
        return true;
    }
    // "Any DNS name that can be constructed by simply adding zero or more
    // labels to the left-hand side" — so the boundary must be a label
    // boundary. `notexample.com` is not inside `example.com`.
    match name.len().checked_sub(subtree.len()) {
        Some(offset) if offset > 0 => {
            name.as_bytes()[offset - 1] == b'.' && eq_ignore_case(&name[offset..], subtree)
        }
        _ => false,
    }
}

fn ip_within(address: &[u8], mask: &[u8], candidate: &[u8]) -> bool {
    if candidate.len() != address.len() {
        // An IPv4 name is never inside an IPv6 subtree, or the reverse.
        return false;
    }
    candidate
        .iter()
        .zip(address)
        .zip(mask)
        .all(|((&value, &base), &mask)| value & mask == base & mask)
}

/// Check one certificate's names against one CA's constraints.
fn check_one(
    certificate: &Certificate<'_>,
    constraints: &Constraints<'_>,
) -> Result<(), NameError> {
    let has_permitted_dns = constraints
        .permitted
        .iter()
        .any(|s| matches!(s, Subtree::Dns(_)));
    let has_permitted_ip = constraints
        .permitted
        .iter()
        .any(|s| matches!(s, Subtree::Ip { .. }));

    for entry in certificate.extensions().subject_alt_names() {
        // A malformed SAN entry cannot match anything and so cannot be
        // permitted either. Skipping it is safe: `verify_server_name` skips
        // it too, so it can never authenticate the connection.
        let Ok(name) = entry else { continue };

        match name {
            GeneralName::DnsName(presented) => {
                for subtree in &constraints.excluded {
                    if let Subtree::Dns(base) = subtree {
                        if dns_within(base, presented) {
                            return Err(NameError::ExcludedByNameConstraint);
                        }
                    }
                }
                if has_permitted_dns
                    && !constraints.permitted.iter().any(|subtree| match subtree {
                        Subtree::Dns(base) => dns_within(base, presented),
                        Subtree::Ip { .. } => false,
                    })
                {
                    return Err(NameError::NotPermittedByNameConstraint);
                }
            }
            GeneralName::IpAddress(presented) => {
                for subtree in &constraints.excluded {
                    if let Subtree::Ip { address, mask } = subtree {
                        if ip_within(address, mask, presented) {
                            return Err(NameError::ExcludedByNameConstraint);
                        }
                    }
                }
                if has_permitted_ip
                    && !constraints.permitted.iter().any(|subtree| match subtree {
                        Subtree::Ip { address, mask } => ip_within(address, mask, presented),
                        Subtree::Dns(_) => false,
                    })
                {
                    return Err(NameError::NotPermittedByNameConstraint);
                }
            }
            // Other name forms are not constrained here because they are not
            // *matched* here either — nothing in this crate can authenticate
            // a connection with them, so there is nothing for a constraint to
            // protect. Constraints *on* those forms are a different matter and
            // are refused at parse time.
            _ => {}
        }
    }

    Ok(())
}

/// Apply one CA's `NameConstraints` to every certificate below it.
///
/// `subordinates` runs from the constraining CA downward, ending with the
/// end-entity certificate.
///
/// Self-issued subordinates are skipped except when last, per RFC 5280
/// §6.1.4(b): a CA re-issuing to itself is a key rollover, not a step into a
/// new part of the namespace, so constraining it would break renewals.
pub fn check_name_constraints(
    constraining: &Certificate<'_>,
    subordinates: &[&Certificate<'_>],
) -> Result<(), NameError> {
    match constraining.extensions().name_constraints() {
        Some(body) => check_name_constraints_body(body, subordinates),
        None => Ok(()),
    }
}

/// As [`check_name_constraints`], but from a raw `NameConstraints` body.
///
/// Trust anchors need this: [`super::path::TrustAnchor`] carries a name, a
/// key, and constraints rather than a whole certificate, because RFC 5280
/// §6.1 treats an anchor's constraints as *inputs* to validation rather than
/// as something read out of a certificate that is never itself validated.
pub fn check_name_constraints_body(
    body: &[u8],
    subordinates: &[&Certificate<'_>],
) -> Result<(), NameError> {
    let constraints = parse_constraints(body)?;

    for (index, certificate) in subordinates.iter().enumerate() {
        let is_last = index + 1 == subordinates.len();
        if certificate.is_self_issued() && !is_last {
            continue;
        }
        check_one(certificate, &constraints)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards_cover_exactly_one_label() {
        assert!(dns_name_matches("*.example.com", "www.example.com"));
        assert!(!dns_name_matches("*.example.com", "example.com"));
        assert!(!dns_name_matches("*.example.com", "a.b.example.com"));
        assert!(!dns_name_matches("www*.example.com", "www1.example.com"));
        assert!(!dns_name_matches("a.*.example.com", "a.b.example.com"));
        assert!(!dns_name_matches("*.com", "example.com"));
        assert!(!dns_name_matches("*", "example.com"));
    }

    #[test]
    fn dns_subtrees_respect_label_boundaries() {
        assert!(dns_within("example.com", "example.com"));
        assert!(dns_within("example.com", "www.example.com"));
        assert!(!dns_within("example.com", "notexample.com"));
        assert!(!dns_within("example.com", "example.com.evil.test"));
        assert!(dns_within("", "anything.test"));
    }
}

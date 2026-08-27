//! X.509 certificate parsing (RFC 5280 §4).
//!
//! # This module validates nothing
//!
//! That is not a caveat, it is the design. [`Certificate::parse`] answers
//! "what does this certificate say", never "should this certificate be
//! trusted". It does not check a signature, does not look at a clock, does
//! not build a chain, and does not decide whether a name matches. A
//! successfully parsed certificate is an attacker-supplied document that has
//! been given structure — nothing more.
//!
//! The distinction matters because it is exactly the one that gets lost.
//! Code that reads `cert.validity().not_after` and stops there has checked
//! nothing an attacker cannot forge, because an attacker writes that field.
//! Path validation is stage 2b of rusty_tls#25 and does not exist yet; until
//! it does, nothing here should be used to make a trust decision.
//!
//! # What it does do
//!
//! Structural parsing, strictly, with the encoded bytes preserved wherever a
//! later stage will need them:
//!
//! - [`Certificate::tbs_der`] — the exact bytes a signature is computed over.
//!   Re-encoding a parsed structure to recover these is how implementations
//!   end up verifying a signature over something other than what they
//!   parsed, so this is a borrow of the original input.
//! - [`Certificate::issuer`] / [`Certificate::subject`] — encoded `Name`s,
//!   kept encoded because RFC 5280 §7.1 name chaining compares them that way.
//! - [`Certificate::extensions`] — `basicConstraints`, `keyUsage`,
//!   `extendedKeyUsage`, and `subjectAltName` parsed; everything else left
//!   alone but *counted* if it is critical, via
//!   [`Extensions::unhandled_critical`].
//!
//! That last one is the extension handling that actually matters. RFC 5280
//! §6.1.3 requires a validator to reject a certificate carrying a critical
//! extension it does not understand — the extension is marked critical
//! precisely to say "refuse rather than ignore me". A parser that quietly
//! skips unknown extensions hands the validator above it no way to comply.
//! So unknown critical extensions are collected and reported, and the
//! decision is left where it belongs.
//!
//! # Structural rules enforced here
//!
//! Parsing fails, rather than producing a value a caller has to second-guess,
//! when:
//!
//! - the `tbsCertificate.signature` algorithm differs from the outer
//!   `signatureAlgorithm` (RFC 5280 §4.1.1.2). These are two copies of one
//!   fact, and a certificate where they disagree is asking two different
//!   questions of two different readers.
//! - `version` is explicitly encoded as v1 (DER omits `DEFAULT` values), or
//!   extensions appear in a certificate that is not v3, or unique identifiers
//!   appear in a v1.
//! - the same extension `OID` appears twice (RFC 5280 §4.2).
//! - the extensions `SEQUENCE` is present but empty (RFC 5280 §4.1.2.9).
//! - any DER encoding is non-canonical — see [`super::der`], which is where
//!   most of the strictness lives.

use super::der::{DerError, ObjectIdentifier, Reader, Tag};

/// Certificate version, as encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Version {
    /// v1 — no extensions, no unique identifiers. Encoded by omission.
    V1,
    /// v2 — unique identifiers permitted, extensions not.
    V2,
    /// v3 — the only version RFC 5280 allows extensions in.
    V3,
}

/// Everything certificate parsing can refuse.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum X509Error {
    /// The DER underneath was malformed. Carries the specific reason.
    Der(DerError),
    /// `version` held a value other than v1, v2, or v3.
    UnsupportedVersion(u64),
    /// `version` was explicitly encoded as v1, which DER forbids because v1
    /// is the `DEFAULT` and defaults are omitted.
    ExplicitDefaultVersion,
    /// Extensions appeared in a certificate that is not v3.
    ExtensionsBeforeV3(Version),
    /// A unique identifier appeared in a v1 certificate.
    UniqueIdentifierInV1,
    /// The extensions `SEQUENCE` was present but empty.
    EmptyExtensions,
    /// The same extension OID appeared more than once.
    DuplicateExtension,
    /// `tbsCertificate.signature` and the outer `signatureAlgorithm` did not
    /// match byte for byte.
    SignatureAlgorithmMismatch,
    /// A `Time` was not a well-formed `UTCTime` or `GeneralizedTime` in the
    /// profile RFC 5280 §4.1.2.5 requires: four- or two-digit year, seconds
    /// present, `Z` suffix, no fractional part.
    MalformedTime,
    /// A `keyUsage` bit string was longer than the nine defined bits.
    MalformedKeyUsage,
    /// A `basicConstraints` extension was structurally invalid.
    MalformedBasicConstraints,
    /// An `iPAddress` general name was neither 4 nor 16 octets.
    MalformedIpAddress {
        /// The length actually present.
        len: usize,
    },
    /// An `IA5String` general name contained a non-ASCII octet.
    NonAsciiName,
}

impl From<DerError> for X509Error {
    fn from(err: DerError) -> Self {
        Self::Der(err)
    }
}

impl core::fmt::Display for X509Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Der(err) => write!(f, "malformed DER: {err}"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported certificate version {v}"),
            Self::ExplicitDefaultVersion => {
                f.write_str("version v1 was encoded explicitly; DER omits DEFAULT values")
            }
            Self::ExtensionsBeforeV3(v) => {
                write!(f, "extensions are present in a {v:?} certificate")
            }
            Self::UniqueIdentifierInV1 => f.write_str("unique identifier in a v1 certificate"),
            Self::EmptyExtensions => f.write_str("the extensions SEQUENCE is empty"),
            Self::DuplicateExtension => f.write_str("an extension OID appears more than once"),
            Self::SignatureAlgorithmMismatch => {
                f.write_str("tbsCertificate.signature does not match the outer signatureAlgorithm")
            }
            Self::MalformedTime => f.write_str("malformed or out-of-profile time"),
            Self::MalformedKeyUsage => f.write_str("keyUsage has more bits than are defined"),
            Self::MalformedBasicConstraints => f.write_str("malformed basicConstraints"),
            Self::MalformedIpAddress { len } => {
                write!(f, "iPAddress is {len} octets, expected 4 or 16")
            }
            Self::NonAsciiName => f.write_str("IA5String name contains a non-ASCII octet"),
        }
    }
}

impl std::error::Error for X509Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Der(err) => Some(err),
            _ => None,
        }
    }
}

type Result<T> = core::result::Result<T, X509Error>;

/// An `AlgorithmIdentifier`: an OID plus whatever parameters it defines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlgorithmIdentifier<'a> {
    /// The algorithm OID.
    pub oid: ObjectIdentifier<'a>,
    /// The parameters, encoded, or `None` if absent. Not interpreted here —
    /// what they mean depends entirely on `oid`.
    pub parameters: Option<&'a [u8]>,
    /// The whole `AlgorithmIdentifier`, encoded. Used to compare the two
    /// copies RFC 5280 requires to agree.
    pub encoded: &'a [u8],
}

/// A validity period, as seconds since the Unix epoch.
///
/// Both bounds are inclusive per RFC 5280 §4.1.2.5. Nothing here compares
/// them to a clock, or to each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Validity {
    /// `notBefore`, in seconds since 1970-01-01T00:00:00Z.
    pub not_before: i64,
    /// `notAfter`, in seconds since 1970-01-01T00:00:00Z.
    pub not_after: i64,
}

/// A `SubjectPublicKeyInfo`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubjectPublicKeyInfo<'a> {
    /// The key's algorithm.
    pub algorithm: AlgorithmIdentifier<'a>,
    /// The key itself — the `BIT STRING` contents, sign octet removed. Its
    /// interpretation depends on `algorithm`; nothing here decodes it.
    pub key: &'a [u8],
    /// The whole `SubjectPublicKeyInfo`, encoded. This is what a key
    /// identifier is computed over, and what a chain builder compares.
    pub encoded: &'a [u8],
}

/// `basicConstraints` (RFC 5280 §4.2.1.9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BasicConstraints {
    /// Whether this certificate may act as a CA.
    pub is_ca: bool,
    /// How many intermediates may follow it, if constrained.
    ///
    /// Meaningful only when `is_ca` is true; RFC 5280 says it MUST NOT
    /// appear otherwise, and parsing rejects that combination.
    pub path_len_constraint: Option<u64>,
}

/// `keyUsage` (RFC 5280 §4.2.1.3), as a bit set.
///
/// Bit 0 is the most significant bit of the first octet, which is the
/// opposite of what "bit 0" suggests and a reliable source of off-by-one
/// bugs — hence the named accessors rather than a public index.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct KeyUsage(u16);

impl KeyUsage {
    const fn has(self, bit: u8) -> bool {
        self.0 & (0x8000 >> bit) != 0
    }

    /// Bit 0 — the key may verify signatures other than on certificates and
    /// CRLs.
    pub const fn digital_signature(self) -> bool {
        self.has(0)
    }
    /// Bit 1 — `contentCommitment`, formerly `nonRepudiation`.
    pub const fn content_commitment(self) -> bool {
        self.has(1)
    }
    /// Bit 2.
    pub const fn key_encipherment(self) -> bool {
        self.has(2)
    }
    /// Bit 3.
    pub const fn data_encipherment(self) -> bool {
        self.has(3)
    }
    /// Bit 4.
    pub const fn key_agreement(self) -> bool {
        self.has(4)
    }
    /// Bit 5 — the key may sign certificates. The one that decides whether a
    /// CA is a CA.
    pub const fn key_cert_sign(self) -> bool {
        self.has(5)
    }
    /// Bit 6 — the key may sign CRLs.
    pub const fn crl_sign(self) -> bool {
        self.has(6)
    }
    /// Bit 7.
    pub const fn encipher_only(self) -> bool {
        self.has(7)
    }
    /// Bit 8.
    pub const fn decipher_only(self) -> bool {
        self.has(8)
    }
}

impl core::fmt::Debug for KeyUsage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let names = [
            ("digitalSignature", self.digital_signature()),
            ("contentCommitment", self.content_commitment()),
            ("keyEncipherment", self.key_encipherment()),
            ("dataEncipherment", self.data_encipherment()),
            ("keyAgreement", self.key_agreement()),
            ("keyCertSign", self.key_cert_sign()),
            ("cRLSign", self.crl_sign()),
            ("encipherOnly", self.encipher_only()),
            ("decipherOnly", self.decipher_only()),
        ];
        let mut list = f.debug_list();
        for (name, set) in names {
            if set {
                list.entry(&name);
            }
        }
        list.finish()
    }
}

/// One `GeneralName` from a `subjectAltName` extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GeneralName<'a> {
    /// `rfc822Name [1]` — an email address.
    Rfc822Name(&'a str),
    /// `dNSName [2]`.
    ///
    /// Returned verbatim, including any embedded NUL. That is deliberate: a
    /// NUL inside a dNSName is the null-prefix attack (CVE-2009-2408), where
    /// `evil.com\0good.com` reads as `good.com` to anything using C string
    /// semantics. Dropping or rejecting it here would hide the attack from
    /// the name matcher that has to defend against it, so it is preserved and
    /// the matcher is on notice.
    DnsName(&'a str),
    /// `uniformResourceIdentifier [6]`.
    Uri(&'a str),
    /// `iPAddress [7]` — 4 octets for IPv4, 16 for IPv6.
    IpAddress(&'a [u8]),
    /// A name form this parser does not interpret, kept whole.
    Other {
        /// The context-specific tag octet it carried.
        tag: u8,
        /// Its contents, uninterpreted.
        contents: &'a [u8],
    },
}

/// The parsed extensions this module understands, plus a record of the
/// critical ones it does not.
#[derive(Clone, Debug, Default)]
pub struct Extensions<'a> {
    basic_constraints: Option<BasicConstraints>,
    key_usage: Option<KeyUsage>,
    extended_key_usage: Option<&'a [u8]>,
    subject_alt_name: Option<&'a [u8]>,
    name_constraints: Option<&'a [u8]>,
    unhandled_critical: Vec<ObjectIdentifier<'a>>,
}

impl<'a> Extensions<'a> {
    /// `basicConstraints`, if present.
    pub const fn basic_constraints(&self) -> Option<BasicConstraints> {
        self.basic_constraints
    }

    /// `keyUsage`, if present.
    pub const fn key_usage(&self) -> Option<KeyUsage> {
        self.key_usage
    }

    /// `subjectAltName` entries, or an empty iterator if the extension is
    /// absent.
    ///
    /// Yields `Result` per entry rather than failing the whole certificate:
    /// one unparseable name should not make the others unavailable to a
    /// caller that only needs a different one.
    pub fn subject_alt_names(&self) -> GeneralNames<'a> {
        GeneralNames {
            reader: Reader::new(self.subject_alt_name.unwrap_or(&[])),
            stopped: false,
        }
    }

    /// True if a `subjectAltName` extension was present at all, which is a
    /// different question from whether it yielded any names.
    pub const fn has_subject_alt_name(&self) -> bool {
        self.subject_alt_name.is_some()
    }

    /// `extendedKeyUsage` OIDs, or an empty iterator if absent.
    pub fn extended_key_usage(&self) -> ExtendedKeyUsage<'a> {
        ExtendedKeyUsage {
            reader: Reader::new(self.extended_key_usage.unwrap_or(&[])),
            stopped: false,
        }
    }

    /// True if an `extendedKeyUsage` extension was present.
    ///
    /// Absent means "no restriction"; present-but-not-listing-your-purpose
    /// means "forbidden". Those are opposite answers, so the distinction
    /// cannot be collapsed into an empty iterator.
    pub const fn has_extended_key_usage(&self) -> bool {
        self.extended_key_usage.is_some()
    }

    /// The raw `NameConstraints` contents, if present.
    ///
    /// Left uninterpreted here for the same reason `subjectAltName` is:
    /// deciding what a constraint *means* is a validator's job, and this
    /// module does not validate. [`super::name`] evaluates it.
    ///
    /// Recognising the extension has a consequence worth being explicit
    /// about: before this was understood, a name-constrained certificate
    /// landed in [`Extensions::unhandled_critical`] and every validator
    /// refused the chain. Now it does not, so whatever consumes this **must**
    /// actually enforce the constraint. Recognising an extension without
    /// enforcing it is strictly worse than not recognising it.
    pub const fn name_constraints(&self) -> Option<&'a [u8]> {
        self.name_constraints
    }

    /// Critical extensions this parser did not interpret.
    ///
    /// RFC 5280 §6.1.3(f) requires a validator to **reject** a certificate
    /// with any entry here. Parsing deliberately does not do that itself —
    /// the same certificate may be fine for one purpose and not another —
    /// but a validator that ignores this list is not conforming, and is
    /// ignoring an extension whose author marked it "refuse rather than skip
    /// me".
    pub fn unhandled_critical(&self) -> &[ObjectIdentifier<'a>] {
        &self.unhandled_critical
    }
}

/// Iterator over `subjectAltName` entries.
#[derive(Clone, Debug)]
pub struct GeneralNames<'a> {
    reader: Reader<'a>,
    stopped: bool,
}

impl<'a> Iterator for GeneralNames<'a> {
    type Item = Result<GeneralName<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.stopped || self.reader.is_empty() {
            return None;
        }
        let before = self.reader.remaining();
        let item = self.read_one();
        // Some errors leave the cursor exactly where it was — a wrong tag is
        // deliberately not consumed, so that `OPTIONAL` fields work. Yielding
        // that error and going round again would yield it forever. Attacker
        // input decides which error this is, so termination cannot rest on
        // which ones happen to advance.
        if item.is_err() && self.reader.remaining() == before {
            self.stopped = true;
        }
        Some(item)
    }
}

impl<'a> GeneralNames<'a> {
    fn read_one(&mut self) -> Result<GeneralName<'a>> {
        let value = self.reader.read_any()?;
        let contents = value.contents;
        Ok(match value.tag.0 {
            0x81 => GeneralName::Rfc822Name(ia5(contents)?),
            0x82 => GeneralName::DnsName(ia5(contents)?),
            0x86 => GeneralName::Uri(ia5(contents)?),
            0x87 => match contents.len() {
                4 | 16 => GeneralName::IpAddress(contents),
                len => return Err(X509Error::MalformedIpAddress { len }),
            },
            tag => GeneralName::Other { tag, contents },
        })
    }
}

/// `IA5String` is ASCII by definition, so anything above 0x7f is malformed
/// rather than something to transcode.
fn ia5(bytes: &[u8]) -> Result<&str> {
    if bytes.is_ascii() {
        // Every ASCII byte string is valid UTF-8.
        core::str::from_utf8(bytes).map_err(|_| X509Error::NonAsciiName)
    } else {
        Err(X509Error::NonAsciiName)
    }
}

/// Iterator over `extendedKeyUsage` OIDs.
#[derive(Clone, Debug)]
pub struct ExtendedKeyUsage<'a> {
    reader: Reader<'a>,
    stopped: bool,
}

impl<'a> Iterator for ExtendedKeyUsage<'a> {
    type Item = Result<ObjectIdentifier<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.stopped || self.reader.is_empty() {
            return None;
        }
        let before = self.reader.remaining();
        let item = self.reader.read_oid().map_err(X509Error::from);
        // See `GeneralNames::next`: a value whose tag is not `OBJECT
        // IDENTIFIER` is left unconsumed, so without this the iterator spins
        // on it forever. This is the bug the fuzzer found.
        if item.is_err() && self.reader.remaining() == before {
            self.stopped = true;
        }
        Some(item)
    }
}

/// Well-known OIDs, as encoded bodies.
///
/// Encoded rather than dotted because every use is an equality test, and
/// comparing the bytes that were on the wire avoids a decode step that could
/// itself be wrong.
pub mod oid {
    use super::ObjectIdentifier;

    /// `id-ce-subjectKeyIdentifier`, 2.5.29.14.
    pub const SUBJECT_KEY_IDENTIFIER: ObjectIdentifier<'static> =
        ObjectIdentifier(&[0x55, 0x1d, 0x0e]);
    /// `id-ce-keyUsage`, 2.5.29.15.
    pub const KEY_USAGE: ObjectIdentifier<'static> = ObjectIdentifier(&[0x55, 0x1d, 0x0f]);
    /// `id-ce-subjectAltName`, 2.5.29.17.
    pub const SUBJECT_ALT_NAME: ObjectIdentifier<'static> = ObjectIdentifier(&[0x55, 0x1d, 0x11]);
    /// `id-ce-basicConstraints`, 2.5.29.19.
    pub const BASIC_CONSTRAINTS: ObjectIdentifier<'static> = ObjectIdentifier(&[0x55, 0x1d, 0x13]);
    /// `id-ce-nameConstraints`, 2.5.29.30.
    pub const NAME_CONSTRAINTS: ObjectIdentifier<'static> = ObjectIdentifier(&[0x55, 0x1d, 0x1e]);
    /// `id-ce-certificatePolicies`, 2.5.29.32.
    pub const CERTIFICATE_POLICIES: ObjectIdentifier<'static> =
        ObjectIdentifier(&[0x55, 0x1d, 0x20]);
    /// `id-ce-authorityKeyIdentifier`, 2.5.29.35.
    pub const AUTHORITY_KEY_IDENTIFIER: ObjectIdentifier<'static> =
        ObjectIdentifier(&[0x55, 0x1d, 0x23]);
    /// `id-ce-extKeyUsage`, 2.5.29.37.
    pub const EXTENDED_KEY_USAGE: ObjectIdentifier<'static> = ObjectIdentifier(&[0x55, 0x1d, 0x25]);

    /// `anyExtendedKeyUsage`, 2.5.29.37.0.
    pub const EKU_ANY: ObjectIdentifier<'static> = ObjectIdentifier(&[0x55, 0x1d, 0x25, 0x00]);
    /// `id-kp-serverAuth`, 1.3.6.1.5.5.7.3.1.
    pub const EKU_SERVER_AUTH: ObjectIdentifier<'static> =
        ObjectIdentifier(&[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01]);
    /// `id-kp-clientAuth`, 1.3.6.1.5.5.7.3.2.
    pub const EKU_CLIENT_AUTH: ObjectIdentifier<'static> =
        ObjectIdentifier(&[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x02]);

    /// `rsaEncryption`, 1.2.840.113549.1.1.1.
    pub const RSA_ENCRYPTION: ObjectIdentifier<'static> =
        ObjectIdentifier(&[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01]);
    /// `id-ecPublicKey`, 1.2.840.10045.2.1.
    pub const EC_PUBLIC_KEY: ObjectIdentifier<'static> =
        ObjectIdentifier(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01]);
    /// `id-Ed25519`, 1.3.101.112.
    pub const ED25519: ObjectIdentifier<'static> = ObjectIdentifier(&[0x2b, 0x65, 0x70]);
}

/// A parsed X.509 certificate, borrowing the DER it was parsed from.
///
/// Read this module's header before using any of it — in particular the part
/// about validating nothing.
#[derive(Clone, Debug)]
pub struct Certificate<'a> {
    tbs_der: &'a [u8],
    version: Version,
    serial: &'a [u8],
    signature_algorithm: AlgorithmIdentifier<'a>,
    issuer: &'a [u8],
    validity: Validity,
    subject: &'a [u8],
    spki: SubjectPublicKeyInfo<'a>,
    extensions: Extensions<'a>,
    signature: &'a [u8],
}

impl<'a> Certificate<'a> {
    /// Parse one DER-encoded certificate, which must be the entire input.
    ///
    /// Trailing bytes are an error rather than something to ignore: a second
    /// certificate concatenated onto the first is a thing attackers try, and
    /// "there was more data and I did not look at it" is not a safe silence.
    pub fn parse(der: &'a [u8]) -> Result<Self> {
        let mut outer = Reader::new(der);
        let mut certificate = outer.read_sequence()?;
        outer.finish()?;

        let tbs_value = certificate.read(Tag::SEQUENCE)?;
        let mut tbs = Reader::new(tbs_value.contents);

        // version [0] EXPLICIT INTEGER DEFAULT v1
        let version = match tbs.read_optional(Tag::context(0, true))? {
            None => Version::V1,
            Some(wrapper) => {
                let mut inner = Reader::new(wrapper.contents);
                let raw = inner.read_u64()?;
                inner.finish()?;
                match raw {
                    0 => return Err(X509Error::ExplicitDefaultVersion),
                    1 => Version::V2,
                    2 => Version::V3,
                    other => return Err(X509Error::UnsupportedVersion(other)),
                }
            }
        };

        let serial = tbs.read_unsigned_integer()?;
        let signature_algorithm = read_algorithm_identifier(&mut tbs)?;
        let issuer = tbs.read(Tag::SEQUENCE)?.encoded;
        let validity = read_validity(&mut tbs)?;
        let subject = tbs.read(Tag::SEQUENCE)?.encoded;
        let spki = read_spki(&mut tbs)?;

        // issuerUniqueID [1] and subjectUniqueID [2], v2 and v3 only.
        let has_unique_ids = tbs.read_optional(Tag::context(1, false))?.is_some()
            | tbs.read_optional(Tag::context(2, false))?.is_some();
        if has_unique_ids && version == Version::V1 {
            return Err(X509Error::UniqueIdentifierInV1);
        }

        // extensions [3] EXPLICIT Extensions, v3 only.
        let extensions = match tbs.read_optional(Tag::context(3, true))? {
            None => Extensions::default(),
            Some(wrapper) => {
                if version != Version::V3 {
                    return Err(X509Error::ExtensionsBeforeV3(version));
                }
                let mut inner = Reader::new(wrapper.contents);
                let list = inner.read_sequence()?;
                inner.finish()?;
                read_extensions(list)?
            }
        };
        tbs.finish()?;

        let outer_algorithm = read_algorithm_identifier(&mut certificate)?;
        if outer_algorithm.encoded != signature_algorithm.encoded {
            return Err(X509Error::SignatureAlgorithmMismatch);
        }

        let signature = certificate.read_bit_string_octets()?;
        certificate.finish()?;

        Ok(Self {
            tbs_der: tbs_value.encoded,
            version,
            serial,
            signature_algorithm,
            issuer,
            validity,
            subject,
            spki,
            extensions,
            signature,
        })
    }

    /// The encoded `tbsCertificate` — the exact bytes the signature covers.
    pub const fn tbs_der(&self) -> &'a [u8] {
        self.tbs_der
    }

    /// The certificate version.
    pub const fn version(&self) -> Version {
        self.version
    }

    /// The serial number's unsigned big-endian magnitude, sign octet removed.
    ///
    /// Not an integer type: RFC 5280 permits 20 octets, and every use is a
    /// comparison rather than arithmetic.
    pub const fn serial(&self) -> &'a [u8] {
        self.serial
    }

    /// The signature algorithm, which parsing has already confirmed is
    /// identical in both places the certificate states it.
    pub const fn signature_algorithm(&self) -> AlgorithmIdentifier<'a> {
        self.signature_algorithm
    }

    /// The signature bits, for a verifier that has yet to be written.
    pub const fn signature(&self) -> &'a [u8] {
        self.signature
    }

    /// The encoded `issuer` `Name`.
    pub const fn issuer(&self) -> &'a [u8] {
        self.issuer
    }

    /// The encoded `subject` `Name`.
    pub const fn subject(&self) -> &'a [u8] {
        self.subject
    }

    /// The validity period. Compared to nothing by this module.
    pub const fn validity(&self) -> Validity {
        self.validity
    }

    /// The subject public key.
    pub const fn subject_public_key_info(&self) -> SubjectPublicKeyInfo<'a> {
        self.spki
    }

    /// The extensions, parsed where understood and counted where critical.
    pub const fn extensions(&self) -> &Extensions<'a> {
        &self.extensions
    }

    /// True when this certificate's `subject` equals its `issuer`.
    ///
    /// Names a *shape*, not a fact: a certificate can claim to be its own
    /// issuer without being self-signed, because nothing here verifies the
    /// signature. Useful for chain building, useless for trust.
    pub fn is_self_issued(&self) -> bool {
        self.issuer == self.subject
    }
}

fn read_algorithm_identifier<'a>(reader: &mut Reader<'a>) -> Result<AlgorithmIdentifier<'a>> {
    let value = reader.read(Tag::SEQUENCE)?;
    let mut inner = Reader::new(value.contents);
    let oid = inner.read_oid()?;
    let parameters = if inner.is_empty() {
        None
    } else {
        Some(inner.read_any()?.encoded)
    };
    inner.finish()?;
    Ok(AlgorithmIdentifier {
        oid,
        parameters,
        encoded: value.encoded,
    })
}

fn read_spki<'a>(reader: &mut Reader<'a>) -> Result<SubjectPublicKeyInfo<'a>> {
    let value = reader.read(Tag::SEQUENCE)?;
    let mut inner = Reader::new(value.contents);
    let algorithm = read_algorithm_identifier(&mut inner)?;
    let key = inner.read_bit_string_octets()?;
    inner.finish()?;
    Ok(SubjectPublicKeyInfo {
        algorithm,
        key,
        encoded: value.encoded,
    })
}

fn read_validity(reader: &mut Reader<'_>) -> Result<Validity> {
    let mut inner = reader.read_sequence()?;
    let not_before = read_time(&mut inner)?;
    let not_after = read_time(&mut inner)?;
    inner.finish()?;
    Ok(Validity {
        not_before,
        not_after,
    })
}

/// RFC 5280 §4.1.2.5: `UTCTime` through 2049, `GeneralizedTime` from 2050,
/// both in `Z` with seconds and without a fractional part.
fn read_time(reader: &mut Reader<'_>) -> Result<i64> {
    let value = reader.read_any()?;
    let bytes = value.contents;
    let (year, rest) = match value.tag {
        Tag::UTC_TIME => {
            // YYMMDDHHMMSSZ
            if bytes.len() != 13 {
                return Err(X509Error::MalformedTime);
            }
            let two = digits(&bytes[0..2])?;
            // §4.1.2.5.1: 50..99 is 19xx, 00..49 is 20xx.
            let year = if two >= 50 { 1900 + two } else { 2000 + two };
            (year, &bytes[2..])
        }
        Tag::GENERALIZED_TIME => {
            // YYYYMMDDHHMMSSZ
            if bytes.len() != 15 {
                return Err(X509Error::MalformedTime);
            }
            (digits(&bytes[0..4])?, &bytes[4..])
        }
        _ => return Err(X509Error::MalformedTime),
    };

    if rest[rest.len() - 1] != b'Z' {
        return Err(X509Error::MalformedTime);
    }
    let month = digits(&rest[0..2])?;
    let day = digits(&rest[2..4])?;
    let hour = digits(&rest[4..6])?;
    let minute = digits(&rest[6..8])?;
    let second = digits(&rest[8..10])?;

    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(X509Error::MalformedTime);
    }

    let days = days_from_civil(i64::from(year), i64::from(month), i64::from(day));
    Ok(days * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second))
}

fn digits(bytes: &[u8]) -> Result<u32> {
    let mut value = 0u32;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return Err(X509Error::MalformedTime);
        }
        value = value * 10 + u32::from(b - b'0');
    }
    Ok(value)
}

const fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

const fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
///
/// Howard Hinnant's `days_from_civil`, which is exact for the whole range
/// certificates can express and needs no calendar dependency.
const fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn read_extensions(mut list: Reader<'_>) -> Result<Extensions<'_>> {
    if list.is_empty() {
        return Err(X509Error::EmptyExtensions);
    }

    let mut extensions = Extensions::default();
    let mut seen: Vec<ObjectIdentifier<'_>> = Vec::new();

    while !list.is_empty() {
        let mut extension = list.read_sequence()?;
        let id = extension.read_oid()?;
        let critical = match extension.read_optional(Tag::BOOLEAN)? {
            // DER omits DEFAULT FALSE, so an explicit `false` is malformed —
            // but rejecting it would turn away certificates that exist in the
            // wild, and the value is unambiguous either way. Read it.
            Some(value) => match value.contents {
                [0x00] => false,
                [0xff] => true,
                _ => return Err(X509Error::Der(DerError::NonCanonicalBoolean(0))),
            },
            None => false,
        };
        let contents = extension.read(Tag::OCTET_STRING)?.contents;
        extension.finish()?;

        if seen.contains(&id) {
            return Err(X509Error::DuplicateExtension);
        }
        seen.push(id);

        let mut inner = Reader::new(contents);
        match id {
            oid::BASIC_CONSTRAINTS => {
                extensions.basic_constraints = Some(read_basic_constraints(&mut inner)?);
                inner.finish()?;
            }
            oid::KEY_USAGE => {
                let (bits, _unused) = inner.read_bit_string_flags()?;
                inner.finish()?;
                if bits.len() > 2 {
                    return Err(X509Error::MalformedKeyUsage);
                }
                let mut value = 0u16;
                if let Some(&first) = bits.first() {
                    value |= u16::from(first) << 8;
                }
                if let Some(&second) = bits.get(1) {
                    value |= u16::from(second);
                }
                extensions.key_usage = Some(KeyUsage(value));
            }
            oid::EXTENDED_KEY_USAGE => {
                let purposes = inner.read(Tag::SEQUENCE)?;
                inner.finish()?;
                extensions.extended_key_usage = Some(purposes.contents);
            }
            oid::SUBJECT_ALT_NAME => {
                let names = inner.read(Tag::SEQUENCE)?;
                inner.finish()?;
                extensions.subject_alt_name = Some(names.contents);
            }
            oid::NAME_CONSTRAINTS => {
                let constraints = inner.read(Tag::SEQUENCE)?;
                inner.finish()?;
                extensions.name_constraints = Some(constraints.contents);
            }
            unknown => {
                if critical {
                    extensions.unhandled_critical.push(unknown);
                }
            }
        }
    }

    Ok(extensions)
}

fn read_basic_constraints(reader: &mut Reader<'_>) -> Result<BasicConstraints> {
    let mut inner = reader.read_sequence()?;
    let is_ca = match inner.read_optional(Tag::BOOLEAN)? {
        Some(value) => match value.contents {
            [0x00] => false,
            [0xff] => true,
            _ => return Err(X509Error::MalformedBasicConstraints),
        },
        None => false,
    };
    let path_len_constraint = if inner.is_empty() {
        None
    } else {
        Some(inner.read_u64()?)
    };
    inner.finish()?;

    // RFC 5280 §4.2.1.9: pathLenConstraint is meaningful only for a CA, and
    // "MUST NOT appear" otherwise. A non-CA carrying one is stating a
    // constraint on a chain it can never be part of — malformed, not
    // harmless.
    if path_len_constraint.is_some() && !is_ca {
        return Err(X509Error::MalformedBasicConstraints);
    }

    Ok(BasicConstraints {
        is_ca,
        path_len_constraint,
    })
}

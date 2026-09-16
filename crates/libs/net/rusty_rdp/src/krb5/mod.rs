//! Kerberos v5 (RFC 4120 / MS-KILE) support, std-only.
//!
//! RDP's Network Level Authentication can carry Kerberos instead of NTLM. This
//! module holds the Kerberos pieces, built bottom-up like the rest of the
//! crate:
//!
//! * [`crypto`] — the RC4-HMAC encryption profile (etype 23), which reuses the
//!   crate's MD4/MD5/HMAC-MD5/RC4 primitives. The string-to-key is the NTLM NT
//!   hash, so a Kerberos ticket for a workgroup-style account shares its key
//!   material with NTLM.
//! * [`aes`] — the modern AES encryption profiles (etypes 17/18,
//!   `aes*-cts-hmac-sha1-96`): n-fold, DK/DR key derivation, the PBKDF2
//!   string-to-key, AES-CBC-CTS, and the HMAC-SHA1-96 checksum. These are what
//!   current KDCs prefer.
//! * [`asn1`] — the Kerberos DER building blocks (`[APPLICATION]`/context tags,
//!   signed `Int32`, `GeneralString`, `KerberosTime`, `KerberosFlags`) and the
//!   small shared structures ([`asn1::PrincipalName`], [`asn1::EncryptedData`],
//!   [`asn1::EncryptionKey`], [`asn1::Checksum`]).
//! * [`messages`] — the message PDUs: [`messages::Ticket`],
//!   [`messages::Authenticator`], [`messages::ApReq`], the KDC exchange
//!   ([`messages::KdcReq`] / [`messages::KdcRep`] with [`messages::PaData`]),
//!   [`messages::EncKdcRepPart`], and [`messages::KrbError`].
//! * [`gss`] — the GSS-API / SPNEGO wrapping (RFC 2743 / 4178) that carries the
//!   Kerberos `AP-REQ` inside CredSSP's `negoTokens`: DER OID encoding, the
//!   `InitialContextToken` framing, and SPNEGO `NegTokenInit` / `NegTokenResp`.
//! * [`cfx`] — the RFC 4121 per-message tokens: [`cfx::wrap`] / [`cfx::unwrap`]
//!   (sealing) and [`cfx::mic`] / [`cfx::verify_mic`] (integrity), which
//!   protect the CredSSP public key and credentials with the Kerberos session
//!   key the way NTLM's `EncryptMessage` does.
//! * [`kdc`] — the KDC network client: [`kdc::get_tgt`] drives the
//!   Authentication Service (AS) exchange and [`kdc::tgs_exchange`] the
//!   Ticket-Granting Service (TGS) exchange, both over TCP;
//!   [`kdc::fetch_ap_req`] chains AS, TGS, and local AP-REQ assembly to go
//!   straight from a realm/username/password/service-principal to the
//!   `(ap_req_bytes, session_key)` pair `crate::tls::connect_tls_kerberos`
//!   (feature `tls`) takes — the client-side Kerberos path is wired end to
//!   end, from a plaintext password to an authenticated RDP session, with no
//!   external `kinit`/keytab needed.
//!
//! The server side only speaks NTLM ([`crate::credssp::CredSspServer`]) —
//! there is no Kerberos-accepting counterpart, since validating an `AP-REQ`
//! needs a keytab (or equivalent long-term key access) and a much larger
//! surface than this crate implements.
//!
//! ## Security warning
//!
//! RC4-HMAC (etype 23) is deprecated and weak; it exists only for
//! interoperability. Prefer the AES profiles.

pub mod aes;
pub mod asn1;
pub mod cfx;
pub mod crypto;
pub mod gss;
pub mod kdc;
pub mod messages;

pub use aes::{AesKey, ETYPE_AES128_CTS_HMAC_SHA1_96, ETYPE_AES256_CTS_HMAC_SHA1_96};
pub use asn1::{Checksum, EncryptedData, EncryptionKey, KerberosTime, PrincipalName};
pub use crypto::{CKSUMTYPE_HMAC_MD5, ETYPE_RC4_HMAC};
pub use messages::{ApReq, Authenticator, KdcRep, KdcReq, KrbError, Ticket};

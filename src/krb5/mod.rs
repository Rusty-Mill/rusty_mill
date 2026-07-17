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
//! * [`asn1`] — the Kerberos DER building blocks (`[APPLICATION]`/context tags,
//!   signed `Int32`, `GeneralString`, `KerberosTime`, `KerberosFlags`) and the
//!   small shared structures ([`asn1::PrincipalName`], [`asn1::EncryptedData`],
//!   [`asn1::EncryptionKey`], [`asn1::Checksum`]).
//! * [`messages`] — the message PDUs: [`messages::Ticket`],
//!   [`messages::Authenticator`], [`messages::ApReq`], the KDC exchange
//!   ([`messages::KdcReq`] / [`messages::KdcRep`] with [`messages::PaData`]),
//!   [`messages::EncKdcRepPart`], and [`messages::KrbError`].
//!
//! Still to come: the KDC transport, the AES encryption types (17/18), and the
//! SPNEGO + GSS-API wrapping that plugs Kerberos into [`crate::credssp`].
//!
//! ## Security warning
//!
//! Only RC4-HMAC is implemented so far; it is deprecated and weak. It exists
//! for interoperability with deployments that still accept it.

pub mod asn1;
pub mod crypto;
pub mod messages;

pub use asn1::{Checksum, EncryptedData, EncryptionKey, KerberosTime, PrincipalName};
pub use crypto::{CKSUMTYPE_HMAC_MD5, ETYPE_RC4_HMAC};
pub use messages::{ApReq, Authenticator, KdcRep, KdcReq, KrbError, Ticket};

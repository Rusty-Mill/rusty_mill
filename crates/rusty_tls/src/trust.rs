use std::sync::Arc;

use rustls::client::{WantsClientCert, WebPkiServerVerifier};
use rustls::pki_types::{CertificateDer, CertificateRevocationListDer, PrivateKeyDer};
use rustls::{ClientConfig, ConfigBuilder, RootCertStore};

use crate::danger::NoServerCertVerification;
use crate::error::Error;
use crate::provider::ring_provider;

/// The entry point every `ClientConfig` in this file builds from, instead
/// of `ClientConfig::builder()`'s ambient provider lookup — see
/// `provider.rs` for why.
fn client_config_entry() -> rustls::ConfigBuilder<ClientConfig, rustls::WantsVerifier> {
    ClientConfig::builder_with_provider(ring_provider())
        .with_safe_default_protocol_versions()
        .expect("ring supports the safe default protocol versions")
}

/// How a [`TlsStream`](crate::TlsStream) decides whether to trust the
/// server it connects to. Verify-by-default: [`TrustPolicy::System`] is the
/// only variant [`Default`] produces, and the unsafe variant is named so it
/// reads as dangerous at every call site.
///
/// `#[non_exhaustive]`: new variants get added as this crate grows (e.g.
/// revocation-checking support) without that being a breaking change for
/// callers every time — match with a wildcard arm (`_ => ...`) rather than
/// exhaustively.
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub enum TrustPolicy {
    /// Verify against the operating system's trust anchors, loaded via
    /// [`platform::security::TrustAnchors`] — Windows' ROOT store, macOS's
    /// Security.framework, or a PEM bundle on Linux and the other BSDs
    /// (honoring `SSL_CERT_FILE` and `SSL_CERT_DIR` first if either is
    /// set).
    ///
    /// That loading used to be `rustls-native-certs`; it moved to
    /// `platform` in rusty_tls#24. Reading a trust store is the one part
    /// of the TLS problem that is pure OS personality rather than
    /// cryptography, so it belongs in the crate this ecosystem keeps OS
    /// personality in — see rustils' `docs/design-discussion-tls.md`.
    /// Nothing about the contract below changed with it.
    ///
    /// **This is a best-effort anchor set, on every platform.** Windows'
    /// ROOT store is lazily populated (enumeration can miss roots the chain
    /// engine would fetch on demand); macOS's anchor enumeration returns
    /// built-in roots but not full keychain trust-settings semantics; a flat
    /// DER list can never express a distrust record on any platform. This
    /// is the same honest contract `rustls-native-certs` itself carries —
    /// this crate does not paper over it.
    ///
    /// Individual anchors that fail to load or parse are skipped silently
    /// (matching real-world trust stores, which routinely contain a few);
    /// only a *total* loss of anchors — zero certificates usable — is a
    /// hard error ([`Error::NoTrustAnchors`]), so a connection never
    /// silently runs with a store that trusts nothing.
    #[default]
    System,
    /// Verify against exactly these caller-supplied root certificates
    /// (DER-encoded), ignoring the OS trust store entirely. For hermetic
    /// tests or a private CA.
    ///
    /// Unlike [`TrustPolicy::System`], a certificate here that fails to
    /// parse is a hard error — the caller named these roots deliberately,
    /// so a bad one is a caller bug worth surfacing, not routine noise to
    /// skip past.
    PinnedAnchors(Vec<CertificateDer<'static>>),
    /// Accept any server certificate, unconditionally. No chain building,
    /// no expiry check, no hostname match — **no protection against an
    /// active man-in-the-middle.**
    ///
    /// Exists for servers that present self-signed certificates and rely on
    /// out-of-band trust (e.g. RDP's typical deployment) — never as a
    /// default, and never silently: every call site naming this variant is
    /// declaring, in the type system, that it isn't verifying its peer.
    DangerNoVerification,
    /// Like [`TrustPolicy::PinnedAnchors`], but additionally reject any
    /// server certificate appearing on one of `crls` (DER-encoded
    /// Certificate Revocation Lists) — CRL-based revocation checking.
    ///
    /// OCSP is deliberately not covered by this variant: it would mean
    /// either this crate making network calls for the first time (to fetch
    /// responses itself) or a separate caller-supplied-staple design —
    /// either way a decision distinct from "check a caller-supplied CRL,"
    /// not bundled in here.
    PinnedAnchorsWithRevocation {
        /// The trusted root certificates, DER-encoded — same contract as
        /// [`TrustPolicy::PinnedAnchors`].
        roots: Vec<CertificateDer<'static>>,
        /// DER-encoded CRLs to check presented certificates against.
        crls: Vec<CertificateRevocationListDer<'static>>,
    },
}

/// Load the OS trust anchors as raw DER, from whichever `platform`
/// backend matches this target.
///
/// One `cfg` arm per backend, mirroring rustils' own `platform-bsd` gate
/// for the BSD arm. The trait is identical across all of them — only the
/// mechanism behind it differs (registry store, framework call, file
/// probing), which is exactly the difference this crate is buying by
/// delegating rather than implementing (rusty_tls#24).
///
/// `platform` distinguishes "this host has no readable trust store" from
/// "the store is empty" and reports the former as
/// [`ErrorKind::NotFound`]; both collapse to [`Error::NoTrustAnchors`]
/// here, because from a caller's perspective they are the same
/// refuse-to-connect outcome and the distinction has no remedy at this
/// layer.
///
/// [`ErrorKind::NotFound`]: platform::error::ErrorKind::NotFound
fn system_anchors() -> Result<Vec<Vec<u8>>, Error> {
    use platform::security::TrustAnchors;

    #[cfg(target_os = "linux")]
    let backend = platform_linux::LinuxTrustAnchors;
    #[cfg(windows)]
    let backend = platform_windows::WindowsTrustAnchors;
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    let backend = platform_bsd::BsdTrustAnchors;

    // No backend for this target. `rustls-native-certs` degraded the same
    // way here (an empty anchor set, then a refused connection); this
    // reaches the identical outcome one step earlier and with a clearer
    // error. Deliberately not a `compile_error!`: every other
    // `TrustPolicy` variant still works on such a target, and refusing to
    // build the whole crate over the one variant that can't would be a
    // larger break than the gap warrants.
    #[cfg(not(any(
        target_os = "linux",
        windows,
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    )))]
    return Err(Error::NoTrustAnchors);

    #[cfg(any(
        target_os = "linux",
        windows,
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    backend.load_anchors().map_err(|_| Error::NoTrustAnchors)
}

/// The part of building a `ClientConfig` that's identical whether the
/// caller ends up presenting a client certificate or not: deciding how to
/// verify the *server's* certificate, per `policy`. Shared by
/// [`build_client_config`] and [`build_client_config_with_identity`] so the
/// trust decision itself only lives in one place.
fn client_config_builder(
    policy: &TrustPolicy,
) -> Result<ConfigBuilder<ClientConfig, WantsClientCert>, Error> {
    Ok(match policy {
        TrustPolicy::System => {
            let mut roots = RootCertStore::empty();
            for der in system_anchors()? {
                // Best-effort per the type's documentation: a handful of
                // unparseable anchors in an OS store is normal, not fatal.
                // `platform` already skips what it cannot read; this skips
                // what rustls will not accept.
                let _ = roots.add(CertificateDer::from(der));
            }
            if roots.is_empty() {
                return Err(Error::NoTrustAnchors);
            }
            client_config_entry().with_root_certificates(roots)
        }
        TrustPolicy::PinnedAnchors(certs) => {
            let mut roots = RootCertStore::empty();
            for cert in certs {
                roots.add(cert.clone())?;
            }
            if roots.is_empty() {
                return Err(Error::NoTrustAnchors);
            }
            client_config_entry().with_root_certificates(roots)
        }
        TrustPolicy::DangerNoVerification => client_config_entry()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoServerCertVerification::new())),
        TrustPolicy::PinnedAnchorsWithRevocation { roots, crls } => {
            let mut root_store = RootCertStore::empty();
            for cert in roots {
                root_store.add(cert.clone())?;
            }
            if root_store.is_empty() {
                return Err(Error::NoTrustAnchors);
            }
            let verifier =
                WebPkiServerVerifier::builder_with_provider(Arc::new(root_store), ring_provider())
                    .with_crls(crls.clone())
                    .build()
                    .map_err(|e| Error::InvalidRevocationConfig(e.to_string()))?;
            client_config_entry().with_webpki_verifier(verifier)
        }
    })
}

pub(crate) fn build_client_config(policy: &TrustPolicy) -> Result<Arc<ClientConfig>, Error> {
    let config = client_config_builder(policy)?.with_no_client_auth();
    Ok(Arc::new(config))
}

/// Like [`build_client_config`], but presents `cert_chain`/`key` to the
/// server as a client certificate (mTLS) — for a server that requests and
/// verifies one, rather than the plain `with_no_client_auth()` path.
pub(crate) fn build_client_config_with_identity(
    policy: &TrustPolicy,
    cert_chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> Result<Arc<ClientConfig>, Error> {
    let config = client_config_builder(policy)?.with_client_auth_cert(cert_chain, key)?;
    Ok(Arc::new(config))
}

/// Like [`build_client_config`], but offers `alpn_protocols` during the
/// handshake (`rustls::ClientConfig::alpn_protocols` is a plain field set
/// after building, not part of the typestate builder chain).
pub(crate) fn build_client_config_with_alpn(
    policy: &TrustPolicy,
    alpn_protocols: Vec<Vec<u8>>,
) -> Result<Arc<ClientConfig>, Error> {
    let mut config = client_config_builder(policy)?.with_no_client_auth();
    config.alpn_protocols = alpn_protocols;
    Ok(Arc::new(config))
}

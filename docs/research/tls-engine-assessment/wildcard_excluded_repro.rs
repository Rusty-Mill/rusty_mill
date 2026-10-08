// Reproduction for finding F1 in docs/research/TLS-ENGINE-ASSESSMENT.md.
//
// Not compiled by the workspace. To run: copy to crates/libs/net/rusty_tls/tests/,
// then `RUSTFLAGS='--cfg rusty_tls_handrolled' cargo test -p rusty_tls
// --features handrolled-engine --test <name> -- --nocapture`.
// Observed on 2026-10-08: the native engine returns Ok for "bad.example.com";
// rustls (webpki) returns NameConstraintViolation. When F1 is fixed this should
// become a real test asserting rejection.

#![cfg(all(feature = "handrolled-engine", rusty_tls_handrolled))]
use rcgen::{BasicConstraints, CertificateParams, DnType, GeneralSubtree, IsCa, KeyPair, NameConstraints};
use rusty_tls::handrolled::name::ServerName;
use rusty_tls::handrolled::path::{verify_peer_certificate, PathOptions, TrustAnchor};
use rusty_tls::handrolled::x509::Certificate;

#[test]
fn wildcard_san_versus_excluded_subtree() {
    let root_key = KeyPair::generate().unwrap();
    let mut rp = CertificateParams::new(vec![]).unwrap();
    rp.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    rp.distinguished_name.push(DnType::CommonName, "Scratch Root");
    rp.name_constraints = Some(NameConstraints {
        permitted_subtrees: vec![],
        excluded_subtrees: vec![GeneralSubtree::DnsName("bad.example.com".into())],
    });
    let root = rp.self_signed(&root_key).unwrap();

    let leaf_key = KeyPair::generate().unwrap();
    let lp = CertificateParams::new(vec!["*.example.com".into()]).unwrap();
    let leaf = lp.signed_by(&leaf_key, &root, &root_key).unwrap();

    let root_der = root.der().to_vec();
    let leaf_der = leaf.der().to_vec();
    let root_c = Certificate::parse(&root_der).unwrap();
    let leaf_c = Certificate::parse(&leaf_der).unwrap();
    let anchors = [TrustAnchor::from_certificate(&root_c)];
    let opts = PathOptions { time: 1_767_225_600, ..Default::default() };

    for host in ["ok.example.com", "bad.example.com"] {
        let r = verify_peer_certificate(&leaf_c, &[], &anchors, &ServerName::Dns(host), &opts);
        println!("host={host} -> {r:?}");
    }

    use rustls::client::verify_server_cert_signed_by_trust_anchor;
    use rustls::pki_types::{CertificateDer, UnixTime};
    let mut store = rustls::RootCertStore::empty();
    store.add(CertificateDer::from(root_der.clone())).unwrap();
    let ee = CertificateDer::from(leaf_der.clone());
    let r = verify_server_cert_signed_by_trust_anchor(
        &rustls::server::ParsedCertificate::try_from(&ee).unwrap(),
        &store,
        &[],
        UnixTime::since_unix_epoch(std::time::Duration::from_secs(1_767_225_600)),
        rustls::crypto::ring::default_provider().signature_verification_algorithms.all,
    );
    println!("rustls(webpki) on same chain -> {r:?}");
}

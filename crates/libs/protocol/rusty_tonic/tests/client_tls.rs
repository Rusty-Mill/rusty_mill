//! The client over TLS: trust, mutual TLS, and a refused handshake.
#![allow(clippy::unwrap_used)]

mod client_common;

use client_common::*;
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair,
};
use rustls::pki_types::CertificateDer;
use rusty_hyper::{serve, ClientIdentity, ConnectOptions, Request, Respond, ServeOptions};
use rusty_tls::{TlsAcceptor, TrustPolicy};
use rusty_tonic::client::{Channel, ChannelOptions};
use rusty_tonic::{Code, Scheme, Status};

const PATH: &str = "/pkg.Svc/Method";

/// One CA that signs the server's certificate and, for mutual TLS, the client's.
struct Pki {
    ca: Certificate,
    ca_key: KeyPair,
}

impl Pki {
    fn new(name: &str) -> Pki {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, name);
        params.distinguished_name = dn;
        let ca_key = KeyPair::generate().unwrap();
        let ca = params.self_signed(&ca_key).unwrap();
        Pki { ca, ca_key }
    }

    fn leaf(&self) -> (Vec<u8>, Vec<u8>) {
        let params =
            CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()]).unwrap();
        let key = KeyPair::generate().unwrap();
        let cert = params.signed_by(&key, &self.ca, &self.ca_key).unwrap();
        (cert.der().to_vec(), key.serialize_der())
    }

    fn acceptor(&self, mutual: bool) -> TlsAcceptor {
        let (cert, key) = self.leaf();
        let alpn = vec![b"h2".to_vec()];
        if mutual {
            TlsAcceptor::new_with_client_auth_and_alpn(
                vec![cert],
                key,
                vec![self.ca.der().to_vec()],
                alpn,
            )
        } else {
            TlsAcceptor::new_with_alpn(vec![cert], key, alpn)
        }
        .unwrap()
    }

    fn trust(&self) -> TrustPolicy {
        TrustPolicy::PinnedAnchors(vec![CertificateDer::from(self.ca.der().to_vec())])
    }

    fn identity(&self) -> ClientIdentity {
        let (cert, key) = self.leaf();
        ClientIdentity {
            cert_chain_der: vec![cert],
            key_der: key,
        }
    }
}

/// Answers with the scheme it saw in the message number.
async fn scheme_echo(mut req: Request, respond: Respond) {
    let call = read_call(&mut req).await;
    let n = u32::from(call.head.scheme == Scheme::Https);
    reply(respond, &[blob(n, 2)], &Status::ok()).await;
}

fn channel_to(server: &rusty_hyper::Server, connect: ConnectOptions) -> Channel {
    let options = ChannelOptions {
        connect,
        ..ChannelOptions::default()
    };
    Channel::new(&format!("https://{}", server.local_addr()), options).unwrap()
}

async fn call(channel: &Channel) -> Result<u32, Status> {
    channel
        .unary::<Blob, Blob>(PATH, rusty_tonic::client::Request::new(blob(0, 0)))
        .await
        .map(|r| r.message.n)
}

#[rusty_tokio::test]
async fn a_call_over_tls_says_https() {
    let pki = Pki::new("tonic client test CA");
    let server = serve(
        listener(),
        Some(pki.acceptor(false)),
        ServeOptions::default(),
        scheme_echo,
    )
    .unwrap();
    let connect = ConnectOptions {
        trust: pki.trust(),
        ..ConnectOptions::default()
    };
    assert_eq!(call(&channel_to(&server, connect)).await.unwrap(), 1);
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn a_server_the_client_does_not_trust_is_unavailable() {
    let pki = Pki::new("tonic client test CA");
    let other = Pki::new("some other CA");
    let server = serve(
        listener(),
        Some(pki.acceptor(false)),
        ServeOptions::default(),
        scheme_echo,
    )
    .unwrap();
    let connect = ConnectOptions {
        trust: other.trust(),
        ..ConnectOptions::default()
    };
    let err = call(&channel_to(&server, connect)).await.unwrap_err();
    assert_eq!(err.code(), Code::Unavailable);
    server.shutdown().await;
}

#[rusty_tokio::test]
async fn mutual_tls_admits_a_client_with_a_certificate_and_refuses_one_without() {
    let pki = Pki::new("tonic client test CA");
    let server = serve(
        listener(),
        Some(pki.acceptor(true)),
        ServeOptions::default(),
        scheme_echo,
    )
    .unwrap();

    let with = ConnectOptions {
        trust: pki.trust(),
        identity: Some(pki.identity()),
        ..ConnectOptions::default()
    };
    assert_eq!(call(&channel_to(&server, with)).await.unwrap(), 1);

    let without = ConnectOptions {
        trust: pki.trust(),
        ..ConnectOptions::default()
    };
    let err = call(&channel_to(&server, without)).await.unwrap_err();
    assert_eq!(err.code(), Code::Unavailable);
    server.shutdown().await;
}

//! Shared helpers for the integration tests.
#![allow(dead_code)]

use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType, IsCa, KeyPair,
};
use rusty_hyper::{
    Body, ClientHandle, ClientIdentity, H2Error, HeaderField, Request, Respond, Server,
};
use rusty_tls::{TlsAcceptor, TrustPolicy};
use rusty_tokio::io::TcpListener;

pub fn hf(n: &str, v: &str) -> HeaderField {
    HeaderField::new(n, v)
}

pub fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i.wrapping_mul(31) % 251) as u8).collect()
}

pub fn header(fields: &[HeaderField], name: &str) -> Option<String> {
    fields
        .iter()
        .find(|h| h.name == name.as_bytes())
        .map(|h| String::from_utf8_lossy(&h.value).into_owned())
}

pub fn listener() -> TcpListener {
    TcpListener::bind("127.0.0.1:0".parse().unwrap()).unwrap()
}

/// What a call came back with.
pub struct Reply {
    pub headers: Vec<HeaderField>,
    pub body: Vec<u8>,
    pub trailers: Option<Vec<HeaderField>>,
}

/// Sends one request with `body` and reads the whole response.
pub async fn call(
    h: &ClientHandle,
    scheme: &str,
    extra: &[(&str, String)],
    body: Vec<u8>,
) -> Result<Reply, H2Error> {
    let mut fields = vec![
        hf(":method", "POST"),
        hf(":scheme", scheme),
        hf(":path", "/echo"),
        hf(":authority", "localhost"),
        hf("content-type", "application/grpc"),
    ];
    fields.extend(extra.iter().map(|(n, v)| hf(n, v)));
    let (mut send, resp) = h.send_request(fields, body.is_empty()).await?;
    if !body.is_empty() {
        send.send_data(body, true).await?;
    }
    let mut resp = resp.await?;
    let (mut bytes, mut trailers) = (Vec::new(), None);
    while let Some(piece) = resp.body.next().await {
        match piece? {
            Body::Data(d) => bytes.extend(d),
            Body::Trailers(t) => trailers = Some(t),
        }
    }
    Ok(Reply {
        headers: resp.headers,
        body: bytes,
        trailers,
    })
}

/// Echoes the request body with `grpc-status: 0` trailers, and returns the
/// request's `x-n` header as `x-echo`.
pub async fn echo(mut req: Request, respond: Respond) {
    let n = header(&req.headers, "x-n");
    let mut bytes = Vec::new();
    while let Some(Ok(piece)) = req.body.next().await {
        if let Body::Data(d) = piece {
            bytes.extend(d);
        }
    }
    let mut fields = vec![hf(":status", "200"), hf("content-type", "application/grpc")];
    if let Some(v) = n {
        fields.push(HeaderField::new("x-echo", v));
    }
    let Ok(mut send) = respond.send_response(fields, false).await else {
        return;
    };
    if send.send_data(bytes, false).await.is_err() {
        return;
    }
    let _ = send.send_trailers(vec![hf("grpc-status", "0")]).await;
}

pub fn url(server: &Server, scheme: &str) -> String {
    format!("{scheme}://{}", server.local_addr())
}

pub struct TestCa {
    cert: Certificate,
    key_pair: KeyPair,
}

impl TestCa {
    pub fn generate(common_name: &str) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, common_name);
        params.distinguished_name = dn;
        let key_pair = KeyPair::generate().unwrap();
        let cert = params.self_signed(&key_pair).unwrap();
        TestCa { cert, key_pair }
    }

    pub fn root_der(&self) -> Vec<u8> {
        self.cert.der().to_vec()
    }

    /// A leaf valid for `localhost` and `127.0.0.1`: (certificate, key).
    pub fn issue_leaf(&self) -> (Vec<u8>, Vec<u8>) {
        let params =
            CertificateParams::new(vec!["localhost".to_string(), "127.0.0.1".to_string()]).unwrap();
        let key = KeyPair::generate().unwrap();
        let cert = params.signed_by(&key, &self.cert, &self.key_pair).unwrap();
        (cert.der().to_vec(), key.serialize_der())
    }
}

/// A server CA and a client CA, each with a leaf.
pub struct Pki {
    pub server_ca: TestCa,
    pub client_ca: TestCa,
    server_leaf: (Vec<u8>, Vec<u8>),
    client_leaf: (Vec<u8>, Vec<u8>),
}

pub fn pki() -> Pki {
    let server_ca = TestCa::generate("rusty_hyper test server CA");
    let client_ca = TestCa::generate("rusty_hyper test client CA");
    let server_leaf = server_ca.issue_leaf();
    let client_leaf = client_ca.issue_leaf();
    Pki {
        server_ca,
        client_ca,
        server_leaf,
        client_leaf,
    }
}

impl Pki {
    /// A server that negotiates `alpn` and asks for no client certificate.
    pub fn acceptor(&self, alpn: &[&[u8]]) -> TlsAcceptor {
        TlsAcceptor::new_with_alpn(
            vec![self.server_leaf.0.clone()],
            self.server_leaf.1.clone(),
            alpn.iter().map(|p| p.to_vec()).collect(),
        )
        .unwrap()
    }

    /// A server that negotiates `h2` and requires a client certificate.
    pub fn mtls_acceptor(&self) -> TlsAcceptor {
        TlsAcceptor::new_with_client_auth_and_alpn(
            vec![self.server_leaf.0.clone()],
            self.server_leaf.1.clone(),
            vec![self.client_ca.root_der()],
            vec![b"h2".to_vec()],
        )
        .unwrap()
    }

    pub fn trust(&self) -> TrustPolicy {
        TrustPolicy::PinnedAnchors(vec![rustls::pki_types::CertificateDer::from(
            self.server_ca.root_der(),
        )])
    }

    pub fn identity(&self) -> ClientIdentity {
        ClientIdentity {
            cert_chain_der: vec![self.client_leaf.0.clone()],
            key_der: self.client_leaf.1.clone(),
        }
    }
}

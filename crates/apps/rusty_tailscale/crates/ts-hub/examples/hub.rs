//! Runs a hub for interop checks:
//! `hub <listen addr:port> <advertised host> <preauth key> <cert.pem> <key.pem>`.
//! Control and plain DERP share the TCP port, STUN answers on the same port
//! over UDP, and stock clients' TLS DERP is on the next port.

use rusty_tokio::net::{TcpListener, UdpSocket};
use ts_hub::{Hub, stun};

#[rusty_tokio::main]
async fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [listen, host, key, cert, cert_key] = args.as_slice() else {
        eprintln!(
            "usage: hub <listen addr:port> <advertised host> <preauth key> <cert.pem> <key.pem>"
        );
        std::process::exit(2);
    };
    let tcp = TcpListener::bind(listen.as_str()).await?;
    let port = tcp.local_addr()?.port();
    let udp = UdpSocket::bind(tcp.local_addr()?).await?;
    let tls_addr = std::net::SocketAddr::new(tcp.local_addr()?.ip(), port + 1);
    let tls_tcp = TcpListener::bind(tls_addr).await?;

    let hub = Hub::new();
    hub.add_preauth(key, "mesh");
    hub.advertise(host, port + 1, port);
    let acceptor = acceptor(cert, cert_key)?;
    println!(
        "hub on http://{host}:{port} (STUN udp/{port}, TLS DERP tcp/{})",
        port + 1
    );
    rusty_tokio::spawn(hub.clone().serve_derp_tls(tls_tcp, acceptor));
    rusty_tokio::spawn(stun::serve(udp));
    hub.serve(tcp).await
}

/// A TLS acceptor from a PEM certificate chain and a PKCS#8 PEM key.
fn acceptor(cert: &str, key: &str) -> std::io::Result<rusty_tls::TlsAcceptor> {
    let bad = |e: String| std::io::Error::new(std::io::ErrorKind::InvalidData, e);
    let chain: Vec<Vec<u8>> = pem_blocks(&std::fs::read_to_string(cert)?)
        .map_err(bad)?
        .into_iter()
        .filter(|(label, _)| label == "CERTIFICATE")
        .map(|(_, der)| der)
        .collect();
    let key = pem_blocks(&std::fs::read_to_string(key)?)
        .map_err(bad)?
        .into_iter()
        .find_map(|(label, der)| (label == "PRIVATE KEY").then_some(der))
        .ok_or_else(|| bad("no PKCS#8 PRIVATE KEY block".into()))?;
    rusty_tls::TlsAcceptor::new(chain, key).map_err(|e| bad(e.to_string()))
}

fn pem_blocks(text: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::new();
    let mut open: Option<(String, String)> = None;
    for line in text.lines().map(str::trim) {
        if let Some(label) = line
            .strip_prefix("-----BEGIN ")
            .and_then(|l| l.strip_suffix("-----"))
        {
            open = Some((label.to_string(), String::new()));
        } else if line.starts_with("-----END ") {
            let (label, body) = open.take().ok_or("END without BEGIN")?;
            let der =
                rusty_base64::decode_standard(&body).map_err(|e| format!("{label}: {e:?}"))?;
            out.push((label, der));
        } else if let Some((_, body)) = open.as_mut() {
            body.push_str(line);
        }
    }
    Ok(out)
}

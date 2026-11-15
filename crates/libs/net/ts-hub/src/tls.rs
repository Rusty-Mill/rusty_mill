//! The operator's certificate, from PEM to a [`TlsAcceptor`]. The mux server
//! reads PEM the same way (`rusty_mux_server::pem`); a library cannot depend
//! on that app, and a shared reader in `rusty_tls` is a public-API change for
//! the owner to decide.

use rusty_tls::TlsAcceptor;

/// The DER of every `-----BEGIN label-----` block in `text`, as `(label, der)`.
fn blocks(text: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::new();
    let mut open: Option<(String, String)> = None;
    for line in text.lines().map(str::trim) {
        if let Some(label) = line
            .strip_prefix("-----BEGIN ")
            .and_then(|l| l.strip_suffix("-----"))
        {
            if open.is_some() {
                return Err(format!("{label}: a block begins inside another"));
            }
            open = Some((label.to_string(), String::new()));
        } else if let Some(label) = line
            .strip_prefix("-----END ")
            .and_then(|l| l.strip_suffix("-----"))
        {
            let (begun, body) = open
                .take()
                .filter(|(begun, _)| begun == label)
                .ok_or_else(|| format!("{label}: the block ends without beginning"))?;
            let der = rusty_base64::decode_standard(&body)
                .map_err(|e| format!("{begun}: not base64: {e:?}"))?;
            out.push((begun, der));
        } else if let Some((_, body)) = open.as_mut() {
            body.push_str(line);
        }
    }
    match open {
        Some((label, _)) => Err(format!("{label}: the block is not closed")),
        None => Ok(out),
    }
}

/// A TLS acceptor serving the certificate chain in `cert_pem` with the
/// PKCS#8 key in `key_pem`. Errors say which file is wrong; they never echo
/// key material.
pub fn tls_acceptor(cert_pem: &str, key_pem: &str) -> Result<TlsAcceptor, String> {
    let chain: Vec<_> = blocks(cert_pem)?
        .into_iter()
        .filter(|(label, _)| label == "CERTIFICATE")
        .map(|(_, der)| der)
        .collect();
    if chain.is_empty() {
        return Err("the certificate file has no CERTIFICATE block".into());
    }
    let key = blocks(key_pem)?
        .into_iter()
        .find_map(|(label, der)| (label == "PRIVATE KEY").then_some(der))
        .ok_or("the key file has no PKCS#8 `PRIVATE KEY` block (convert with `openssl pkcs8 -topk8 -nocrypt`)")?;
    TlsAcceptor::new(chain, key).map_err(|e| format!("certificate: {e}"))
}

//! A PEM reader for the TLS listener's certificate and key (#506): just the
//! blocks, as DER. No shared parser exists in the workspace, and a new
//! third-party one is not worth ~20 lines.

/// The DER of every `-----BEGIN label-----` block in `text`, in order, as
/// `(label, der)`. A block that is not closed or not base64 is an error.
pub fn blocks(text: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
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

/// The certificate chain and private key `cert` and `key` hold, as DER.
/// The key is the first `PRIVATE KEY` block (PKCS#8); other key formats are
/// refused with a hint.
pub fn identity(cert: &str, key: &str) -> Result<(Vec<Vec<u8>>, Vec<u8>), String> {
    let chain: Vec<_> = blocks(cert)?
        .into_iter()
        .filter(|(label, _)| label == "CERTIFICATE")
        .map(|(_, der)| der)
        .collect();
    if chain.is_empty() {
        return Err("the certificate file has no CERTIFICATE block".into());
    }
    let key = blocks(key)?
        .into_iter()
        .find_map(|(label, der)| (label == "PRIVATE KEY").then_some(der))
        .ok_or("the key file has no PKCS#8 `PRIVATE KEY` block (convert with `openssl pkcs8 -topk8 -nocrypt`)")?;
    Ok((chain, key))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CERT: &str = "-----BEGIN CERTIFICATE-----\nAQID\n-----END CERTIFICATE-----\n";
    const KEY: &str = "-----BEGIN PRIVATE KEY-----\nBAUG\n-----END PRIVATE KEY-----\n";

    #[test]
    fn blocks_are_read_in_order_with_text_between_ignored() {
        let text = format!("noise\n{CERT}more\n{CERT}");
        let all = blocks(&text).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0], ("CERTIFICATE".to_string(), vec![1, 2, 3]));
    }

    #[test]
    fn an_identity_is_a_chain_and_a_pkcs8_key() {
        assert_eq!(
            identity(CERT, KEY).unwrap(),
            (vec![vec![1, 2, 3]], vec![4, 5, 6])
        );
    }

    #[test]
    fn broken_input_is_refused() {
        for bad in [
            "-----BEGIN CERTIFICATE-----\nAQID\n",
            "-----END CERTIFICATE-----\n",
            "-----BEGIN CERTIFICATE-----\n!!\n-----END CERTIFICATE-----\n",
            "-----BEGIN A-----\n-----BEGIN B-----\n",
        ] {
            assert!(blocks(bad).is_err(), "{bad:?}");
        }
        assert!(identity("", KEY).is_err());
        assert!(identity(CERT, CERT).is_err());
        assert!(
            identity(
                CERT,
                "-----BEGIN RSA PRIVATE KEY-----\nBAUG\n-----END RSA PRIVATE KEY-----\n"
            )
            .is_err()
        );
    }
}

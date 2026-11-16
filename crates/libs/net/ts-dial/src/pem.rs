//! PEM text to DER blocks, for certificates the operator names on a command
//! line. No variant of the error holds file content, so it can be logged
//! without leaking a key: the one byte `rusty_base64` reports for a bad
//! character is dropped.

/// A PEM file that does not parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{file} file: {reason}")]
pub struct PemError {
    pub file: &'static str,
    pub reason: &'static str,
}

/// The DER of every `-----BEGIN label-----` block in `text`, as `(label, der)`.
/// `file` names the input in errors ("certificate", "key", ...).
pub fn blocks(text: &str, file: &'static str) -> Result<Vec<(String, Vec<u8>)>, PemError> {
    let pem = |reason| PemError { file, reason };
    let mut out = Vec::new();
    let mut open: Option<(String, String)> = None;
    for line in text.lines().map(str::trim) {
        if let Some(label) = line
            .strip_prefix("-----BEGIN ")
            .and_then(|l| l.strip_suffix("-----"))
        {
            if open.is_some() {
                return Err(pem("a block begins inside another"));
            }
            open = Some((label.to_string(), String::new()));
        } else if let Some(label) = line
            .strip_prefix("-----END ")
            .and_then(|l| l.strip_suffix("-----"))
        {
            let (begun, body) = open
                .take()
                .filter(|(begun, _)| begun == label)
                .ok_or_else(|| pem("a block ends without beginning"))?;
            let der =
                rusty_base64::decode_standard(&body).map_err(|_| pem("a block is not base64"))?;
            out.push((begun, der));
        } else if let Some((_, body)) = open.as_mut() {
            body.push_str(line);
        }
    }
    match open {
        Some(_) => Err(pem("a block is not closed")),
        None => Ok(out),
    }
}

/// The DER of every `CERTIFICATE` block in `text`.
pub fn certificates(text: &str) -> Result<Vec<Vec<u8>>, PemError> {
    Ok(blocks(text, "certificate")?
        .into_iter()
        .filter(|(label, _)| label == "CERTIFICATE")
        .map(|(_, der)| der)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEM: &str = "junk\n-----BEGIN CERTIFICATE-----\nAQID\n-----END CERTIFICATE-----\n\
-----BEGIN PRIVATE KEY-----\nBAUG\n-----END PRIVATE KEY-----\n";

    #[test]
    fn reads_every_block_and_keeps_labels() {
        let got = blocks(PEM, "test").unwrap();
        assert_eq!(
            got,
            vec![
                ("CERTIFICATE".to_string(), vec![1, 2, 3]),
                ("PRIVATE KEY".to_string(), vec![4, 5, 6])
            ]
        );
        assert_eq!(certificates(PEM).unwrap(), vec![vec![1, 2, 3]]);
    }

    #[test]
    fn rejects_malformed_blocks_without_echoing_content() {
        for (text, reason) in [
            ("-----BEGIN A-----\nAQID\n", "a block is not closed"),
            ("-----END A-----\n", "a block ends without beginning"),
            (
                "-----BEGIN A-----\n-----BEGIN B-----\n",
                "a block begins inside another",
            ),
            (
                "-----BEGIN A-----\n!!!!\n-----END A-----\n",
                "a block is not base64",
            ),
        ] {
            let err = blocks(text, "certificate").unwrap_err();
            assert_eq!(err.reason, reason);
            assert!(!err.to_string().contains("!!!!"));
        }
    }
}

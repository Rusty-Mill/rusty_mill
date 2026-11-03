//! Escapes for text in probe arguments and result lines: one UTF-16 unit
//! at a time, so a result shows exactly what the console holds.

/// UTF-16 units as one word: printable ASCII as itself, the rest escaped.
pub(crate) fn escape(units: &[u16]) -> String {
    let mut out = String::new();
    for &u in units {
        match u {
            0x5c => out.push_str("\\\\"),
            0x20 => out.push_str("\\s"),
            0x0d => out.push_str("\\r"),
            0x0a => out.push_str("\\n"),
            0x09 => out.push_str("\\t"),
            0x08 => out.push_str("\\b"),
            0x1b => out.push_str("\\e"),
            0x21..=0x7e => out.push(char::from(u as u8)),
            _ => out.push_str(&format!("\\u{{{u:x}}}")),
        }
    }
    out
}

/// The units an escaped argument stands for; see [`escape`]. `\xHH` is
/// one unit too.
pub(crate) fn unescape(text: &str) -> Result<Vec<u16>, String> {
    let mut out = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            let mut buf = [0u16; 2];
            out.extend_from_slice(c.encode_utf16(&mut buf));
            continue;
        }
        let unit = match chars.next() {
            Some('\\') => 0x5c,
            Some('s') => 0x20,
            Some('r') => 0x0d,
            Some('n') => 0x0a,
            Some('t') => 0x09,
            Some('b') => 0x08,
            Some('e') => 0x1b,
            Some('x') => {
                let hex: String = chars.by_ref().take(2).collect();
                u16::from_str_radix(&hex, 16).map_err(|_| format!("bad \\x{hex}"))?
            }
            Some('u') => {
                let rest = chars.as_str();
                let body = rest
                    .strip_prefix('{')
                    .and_then(|r| r.split_once('}'))
                    .ok_or("\\u needs {hex}")?;
                let unit = u16::from_str_radix(body.0, 16)
                    .map_err(|_| format!("bad \\u{{{}}}", body.0))?;
                chars = body.1.chars();
                unit
            }
            other => return Err(format!("unknown escape \\{other:?}")),
        };
        out.push(unit);
    }
    Ok(out)
}

/// Bytes from hex digits.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn unhex(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err("an even number of hex digits".to_owned());
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

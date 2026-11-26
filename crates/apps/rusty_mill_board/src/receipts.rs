//! Approval receipts for BBP human gates (#447, D191, D194).
//!
//! An `approve` holder asks the board to vouch that a named person approved one BBP gate: a
//! task's plan or merge, at the revision their view showed. The board signs that with its
//! Ed25519 key, and `bbp human --approval FILE` accepts the answer on an owned task only with a
//! receipt that verifies against the board's public key, which is committed in the repo
//! (`crates/apps/rusty_bbp_host/keys/board-approval.ed25519.pub`).
//!
//! The file is the receipt's JSON, a newline, the hex signature over exactly those bytes, and a
//! newline. Its fields are those of `rusty_bbp_host::approval::Receipt`, in that order.

use std::path::Path;

use rusty_pk::ed25519::SigningKey;
use serde::{Deserialize, Serialize};

/// What the board vouches for. Mirrors `rusty_bbp_host::approval::Receipt`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    /// The BBP task id.
    pub task: String,
    /// `plan` or `merge`.
    pub gate: String,
    /// The approved artifact.
    pub subject: u64,
    /// The approved run: the merge gate only.
    #[serde(default)]
    pub run: Option<u64>,
    /// The task revision the approver's view showed.
    pub fence: u64,
    /// Who approved: the board actor, the BBP answer's label.
    pub approver: String,
    /// The approval in the approver's words.
    pub verbatim: String,
    /// Where those words were said.
    pub source: String,
    /// When the board signed it, Unix milliseconds.
    pub issued_ms: i64,
}

/// The longest verbatim approval a receipt carries, in bytes.
const MAX_VERBATIM: usize = 2000;
/// The longest task id or source, in bytes.
const MAX_SHORT: usize = 400;

impl Receipt {
    /// Refuse a receipt `bbp` would refuse, so none is signed that cannot be used.
    ///
    /// # Errors
    /// The first field that is wrong.
    pub fn check(&self) -> Result<(), &'static str> {
        let one_line =
            |s: &str, max: usize| !s.trim().is_empty() && s.len() <= max && !s.contains('\n');
        if !one_line(&self.task, MAX_SHORT) {
            return Err("task must be one line");
        }
        match (self.gate.as_str(), self.run) {
            ("plan", None) | ("merge", Some(_)) => {}
            ("plan", Some(_)) => return Err("a plan approval names no run"),
            ("merge", None) => return Err("a merge approval names its run"),
            _ => return Err("gate is plan or merge"),
        }
        if !valid_approver(&self.approver) {
            return Err(
                "the approver's name must be 1-32 of letters, digits, - _ . to label a BBP answer",
            );
        }
        if !one_line(&self.source, MAX_SHORT) {
            return Err("source must name where the approval was said, on one line");
        }
        if self.verbatim.trim().is_empty() || self.verbatim.len() > MAX_VERBATIM {
            return Err("verbatim must hold the approval in the approver's words");
        }
        Ok(())
    }
}

/// What `bbp human` accepts as an answer's label.
fn valid_approver(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Sign `r`: the receipt file's bytes.
///
/// # Errors
/// A receipt that fails [`Receipt::check`], or a key that cannot sign.
pub fn issue(key: &SigningKey, r: &Receipt) -> Result<String, String> {
    r.check()?;
    let body = serde_json::to_string(r).map_err(|e| e.to_string())?;
    let sig = key
        .sign(body.as_bytes())
        .map_err(|_| "the approval key cannot sign".to_owned())?;
    Ok(format!("{body}\n{}\n", hex(&sig)))
}

/// Append an issued receipt `file`'s receipt line to the `issued` record (owner-only), the
/// board's audit trail of what it signed.
///
/// # Errors
/// A record that cannot be written: the receipt is then not handed out.
pub fn record(issued: &Path, file: &str) -> Result<(), String> {
    use std::io::Write;
    // One write of the whole line, so receipts issued at once do not interleave in the append.
    let line = format!("{}\n", file.lines().next().unwrap_or_default());
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
        .open(issued)
        .and_then(|mut f| {
            f.write_all(line.as_bytes())?;
            f.sync_data()
        })
        .map_err(|e| format!("could not record the receipt in {}: {e}", issued.display()))
}

/// The record of issued receipts kept beside the key at `key`.
pub fn issued_path(key: &Path) -> std::path::PathBuf {
    let mut name = key.as_os_str().to_owned();
    name.push(".issued.jsonl");
    name.into()
}

/// The public key `bbp` must hold to verify this board's receipts, as hex.
pub fn public_hex(key: &SigningKey) -> String {
    hex(key.public_key())
}

/// Load the key's 32-byte seed from `path` (64 hex characters, owner-only), creating it with a
/// fresh seed when it does not exist.
///
/// # Errors
/// A file that cannot be read or written, is readable by others, or does not hold a seed.
pub fn load_or_create(path: &Path) -> Result<SigningKey, String> {
    let at = |e: std::io::Error| format!("{}: {e}", path.display());
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => create(path)?,
        Err(e) => return Err(at(e)),
    };
    private(path)?;
    let seed =
        unhex32(text.trim()).ok_or_else(|| format!("{}: not a 64-hex seed", path.display()))?;
    SigningKey::from_seed(&seed).map_err(|_| format!("{}: not a usable key", path.display()))
}

fn create(path: &Path) -> Result<String, String> {
    use std::io::Write;
    let mut seed = [0u8; 32];
    rusty_rand::fill(&mut seed).map_err(|e| e.to_string())?;
    let text = hex(&seed);
    seed.fill(0);
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
        .open(path)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(text)
}

#[cfg(unix)]
fn private(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .permissions()
        .mode();
    match mode & 0o077 {
        0 => Ok(()),
        _ => Err(format!("{} must be chmod 600", path.display())),
    }
}

#[cfg(not(unix))]
fn private(_: &Path) -> Result<(), String> {
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (o, c) in out.iter_mut().zip(s.as_bytes().chunks(2)) {
        let hi = (c[0] as char).to_digit(16)?;
        let lo = (c[1] as char).to_digit(16)?;
        *o = u8::try_from(hi * 16 + lo).ok()?;
    }
    Some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn plan() -> Receipt {
        Receipt {
            task: "orch-abc".into(),
            gate: "plan".into(),
            subject: 2,
            run: None,
            fence: 9,
            approver: "nano".into(),
            verbatim: "Approve the plan? Yes.".into(),
            source: "cmsg_x".into(),
            issued_ms: 1,
        }
    }

    #[test]
    fn an_issued_receipt_verifies_over_its_first_line() {
        let key = SigningKey::from_seed(&[5; 32]).unwrap();
        let file = issue(&key, &plan()).unwrap();
        let (body, sig) = file.trim_end().split_once('\n').unwrap();
        let sig: Vec<u8> = (0..sig.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&sig[i..i + 2], 16).unwrap())
            .collect();
        rusty_pk::ed25519::verify(key.public_key(), body.as_bytes(), &sig).unwrap();
        assert_eq!(serde_json::from_str::<Receipt>(body).unwrap(), plan());
    }

    #[test]
    fn a_receipt_bbp_would_refuse_is_not_signed() {
        let key = SigningKey::from_seed(&[5; 32]).unwrap();
        let bad = |f: fn(&mut Receipt)| {
            let mut r = plan();
            f(&mut r);
            issue(&key, &r).unwrap_err()
        };
        assert!(bad(|r| r.approver = "nano proxy".into()).contains("approver"));
        assert!(bad(|r| r.gate = "merge".into()).contains("run"));
        assert!(bad(|r| r.run = Some(1)).contains("no run"));
        assert!(bad(|r| r.verbatim = " ".into()).contains("verbatim"));
        assert!(bad(|r| r.source = String::new()).contains("source"));
    }

    #[test]
    fn the_key_file_is_created_once_and_must_stay_private() {
        let dir = std::env::temp_dir().join(format!("board-approval-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("approval.key");
        let a = load_or_create(&path).unwrap();
        let b = load_or_create(&path).unwrap();
        assert_eq!(
            public_hex(&a),
            public_hex(&b),
            "the same key on every start"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(load_or_create(&path).unwrap_err().contains("chmod 600"));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod format {
    use super::*;

    /// The receipt `bbp` is tested to accept (`rusty_bbp_host` `tests/receipt_format.rs`): the
    /// board's bytes for this receipt under seed `[5; 32]` must stay exactly these.
    const FIXTURE: &str = include_str!("../../rusty_bbp_host/tests/fixtures/board-receipt.txt");

    #[test]
    fn the_board_signs_exactly_the_bytes_bbp_is_tested_against() {
        let key = SigningKey::from_seed(&[5; 32]).unwrap();
        let r = Receipt {
            task: "orch-abc".into(),
            gate: "merge".into(),
            subject: 4,
            run: Some(1),
            fence: 17,
            approver: "nano".into(),
            verbatim: "Merge #12 at abc123? Yes.".into(),
            source: "cmsg_fixture".into(),
            issued_ms: 1_764_000_000_000,
        };
        let file = issue(&key, &r).unwrap();
        if std::env::var_os("WRITE_RECEIPT_FIXTURE").is_some() {
            std::fs::write(
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../rusty_bbp_host/tests/fixtures/board-receipt.txt"
                ),
                &file,
            )
            .unwrap();
        }
        assert_eq!(file, FIXTURE);
        assert_eq!(public_hex(&key).len(), 64);
    }
}

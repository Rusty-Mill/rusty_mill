//! Board-signed approval receipts for an owned task's human gates (#447, D191-D194).
//!
//! On an owned task `approve-plan` and `approve-merge` need a receipt: the board signs, with its
//! Ed25519 key, what an `approve` holder approved (task, gate, artifact, run, the human view's
//! fence) and in whose words. The host checks the signature against the board key compiled into
//! `bbp` ([`BOARD_KEY`]) and the binding against the action, stores the receipt as a task blob,
//! and answers under [`op`], `by:<approver>/rcpt:<sha256>`. `bbp metrics` re-verifies it.
//!
//! A receipt file is two lines: the receipt's JSON, then the hex signature over exactly those
//! bytes. Signing the bytes as written means neither side needs a canonical encoding.

use rusty_bbp::*;
use rusty_serde::{Deserialize, Serialize};

/// The board's public key, 64 hex characters, changed only by a reviewed PR (D194). Empty until
/// the board's key exists: then no receipt verifies and owned approvals are refused.
pub const BOARD_KEY: &str = include_str!("../keys/board-approval.ed25519.pub");

/// What the board vouches for. Field order is the JSON the board writes; the host never
/// re-encodes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub task: String,
    /// `plan` or `merge`.
    pub gate: String,
    /// The approved artifact.
    pub subject: u64,
    /// The approved run: the merge gate only.
    #[rusty_serde(default)]
    pub run: Option<u64>,
    /// The task revision the approver's view showed; the answer must be fenced on it.
    pub fence: u64,
    /// Who approved, 1 to 32 of `[A-Za-z0-9._-]`: the op's label.
    pub approver: String,
    /// The approval in the approver's words.
    pub verbatim: String,
    /// Where those words were said.
    pub source: String,
    pub issued_ms: i64,
}

/// A receipt whose signature verified, with the exact bytes it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    pub receipt: Receipt,
    pub bytes: Vec<u8>,
}

/// The board key from its hex text, or why it is unusable.
///
/// # Errors
/// When the text is not 64 hex characters (an empty key file included).
pub fn key(hex: &str) -> Result<[u8; 32], String> {
    Sha256::from_hex(hex.trim())
        .map(|s| s.0)
        .ok_or_else(|| "no board approval key is installed, so no receipt can verify".to_owned())
}

/// Split, check and parse a receipt file against `key`.
///
/// # Errors
/// A malformed file, a bad signature, or a receipt that does not parse.
pub fn verify(key: &[u8; 32], file: &[u8]) -> Result<Verified, String> {
    let text = std::str::from_utf8(file).map_err(|_| "receipt: not UTF-8".to_owned())?;
    let (body, sig) = text
        .trim_end_matches('\n')
        .rsplit_once('\n')
        .ok_or("receipt: expected the receipt line, then the signature line")?;
    let sig = unhex(sig.trim()).ok_or("receipt: the signature is not hex")?;
    rusty_pk::ed25519::verify(key, body.as_bytes(), &sig)
        .map_err(|_| "receipt: the board's signature does not verify".to_owned())?;
    let receipt: Receipt =
        rusty_serde::json::from_str(body).map_err(|e| format!("receipt: {e}"))?;
    crate::human::labelled_op(&receipt.approver).map_err(|_| "receipt: bad approver name")?;
    Ok(Verified {
        receipt,
        bytes: file.to_vec(),
    })
}

/// Refuse a receipt that vouches for anything but this fenced approval on `task`.
///
/// # Errors
/// The first field that differs.
pub fn binds(
    r: &Receipt,
    task: &TaskId,
    fence: Option<Rev>,
    act: &HumanAction,
) -> Result<(), String> {
    let HumanAction::Approve { gate, subject, run } = act else {
        return Err("receipt: only plan and merge approvals take a receipt".into());
    };
    let want_gate = match gate {
        Gate::Plan => "plan",
        Gate::Merge => "merge",
    };
    let checks = [
        (r.task == task.0, "task"),
        (r.gate == want_gate, "gate"),
        (r.subject == subject.0, "artifact"),
        (r.run == run.map(|r| r.0), "run"),
        (Some(Rev(r.fence)) == fence, "fence"),
    ];
    match checks.iter().find(|(ok, _)| !ok) {
        Some((_, what)) => Err(format!("receipt: its {what} is not this answer's")),
        None => Ok(()),
    }
}

/// The op a receipted answer runs under: the same receipt always gives the same op, so a resend
/// replays (D192).
pub fn op(v: &Verified) -> OpId {
    OpId(format!(
        "by:{}/{RCPT}{}",
        v.receipt.approver,
        Sha256::of(&v.bytes).hex()
    ))
}

const RCPT: &str = "rcpt:";

/// The receipt blob an op names, if it is a receipted op.
pub fn receipt_of(op: &OpId) -> Option<Sha256> {
    op.0.strip_prefix("by:")
        .and_then(|r| r.split_once('/'))
        .and_then(|(_, rest)| rest.strip_prefix(RCPT))
        .and_then(Sha256::from_hex)
}

/// The approval inside a possibly fenced action, with its fence.
pub fn approval_in(act: &HumanAction) -> Option<(&HumanAction, Option<Rev>)> {
    match act {
        HumanAction::Approve { .. } => Some((act, None)),
        HumanAction::Fenced {
            expected_rev,
            action,
        } => approval_in(action).map(|(a, _)| (a, Some(*expected_rev))),
        _ => None,
    }
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    s.as_bytes()
        .chunks(2)
        .map(|c| {
            let hi = (c[0] as char).to_digit(16)?;
            let lo = (c[1] as char).to_digit(16)?;
            u8::try_from(hi * 16 + lo).ok()
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn signer() -> rusty_pk::ed25519::SigningKey {
        rusty_pk::ed25519::SigningKey::from_seed(&[7; 32]).expect("key")
    }

    pub(crate) fn sign(k: &rusty_pk::ed25519::SigningKey, r: &Receipt) -> Vec<u8> {
        let body = rusty_serde::json::to_string(r).expect("json");
        let sig: String = k
            .sign(body.as_bytes())
            .expect("sign")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        format!("{body}\n{sig}\n").into_bytes()
    }

    pub(crate) fn receipt(gate: &str, run: Option<u64>) -> Receipt {
        Receipt {
            task: "T1".into(),
            gate: gate.into(),
            subject: 2,
            run,
            fence: 9,
            approver: "nano".into(),
            verbatim: "Approve the plan? Yes.".into(),
            source: "cmsg_x".into(),
            issued_ms: 1,
        }
    }

    fn plan() -> HumanAction {
        HumanAction::Approve {
            gate: Gate::Plan,
            subject: ArtId(2),
            run: None,
        }
    }

    #[test]
    fn a_signed_receipt_verifies_and_names_its_op() {
        let k = signer();
        let file = sign(&k, &receipt("plan", None));
        let v = verify(k.public_key(), &file).unwrap();
        assert_eq!(v.receipt.approver, "nano");
        let op = op(&v);
        assert!(op.0.starts_with("by:nano/rcpt:"), "{}", op.0);
        assert_eq!(receipt_of(&op), Some(Sha256::of(&file)));
        assert_eq!(crate::human::answered_by(&op), "nano");
    }

    #[test]
    fn a_forged_or_altered_receipt_is_refused() {
        let k = signer();
        let file = sign(&k, &receipt("plan", None));
        let other = rusty_pk::ed25519::SigningKey::from_seed(&[8; 32]).unwrap();
        assert!(verify(other.public_key(), &file).is_err(), "another key");
        let tampered = String::from_utf8(file)
            .unwrap()
            .replace("\"nano\"", "\"evil\"");
        assert!(
            verify(k.public_key(), tampered.as_bytes()).is_err(),
            "edited body"
        );
        assert!(verify(k.public_key(), b"{}").is_err(), "no signature line");
    }

    #[test]
    fn a_receipt_binds_task_gate_artifact_run_and_fence() {
        let r = receipt("plan", None);
        let t = TaskId("T1".into());
        assert!(binds(&r, &t, Some(Rev(9)), &plan()).is_ok());
        let refused = |task: &str, fence: u64, act: &HumanAction| {
            binds(&r, &TaskId(task.into()), Some(Rev(fence)), act).unwrap_err()
        };
        assert!(refused("T2", 9, &plan()).contains("task"));
        assert!(refused("T1", 8, &plan()).contains("fence"));
        assert!(binds(&r, &t, None, &plan()).unwrap_err().contains("fence"));
        let other_art = HumanAction::Approve {
            gate: Gate::Plan,
            subject: ArtId(3),
            run: None,
        };
        assert!(refused("T1", 9, &other_art).contains("artifact"));
        let merge = HumanAction::Approve {
            gate: Gate::Merge,
            subject: ArtId(2),
            run: Some(RunId(1)),
        };
        assert!(refused("T1", 9, &merge).contains("gate"));
        let m = receipt("merge", Some(2));
        assert!(binds(&m, &t, Some(Rev(9)), &merge)
            .unwrap_err()
            .contains("run"));
    }

    #[test]
    fn an_empty_key_file_is_no_key() {
        assert!(key("").is_err());
        assert!(key(&"ab".repeat(32)).is_ok());
    }
}

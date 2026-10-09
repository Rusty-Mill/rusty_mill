//! Records: principals, artifacts, messages and their typed payloads.

use crate::ids::*;
use rusty_serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Role {
    Planner,
    Coder,
    Tester,
    Reviewer,
}

impl Role {
    pub const ALL: [Role; 4] = [Role::Planner, Role::Coder, Role::Tester, Role::Reviewer];
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrincipalKind {
    Human,
    Moderator,
    Runner,
    Agent { role: Role, vendor: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    pub id: PrincipalId,
    pub kind: PrincipalKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ArtifactKind {
    Brief,
    Spec,
    Diff,
    Candidate,
    TestReport,
    Log,
}

/// Typed artifact contents, decoded by the adapter from the exact bytes stored (A1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactPayload {
    Brief,
    /// A spec must reference the task brief.
    Spec {
        brief: ArtId,
    },
    Diff,
    Candidate(Candidate),
    TestReport(Report),
    Log,
}

impl ArtifactPayload {
    pub fn kind(&self) -> ArtifactKind {
        match self {
            ArtifactPayload::Brief => ArtifactKind::Brief,
            ArtifactPayload::Spec { .. } => ArtifactKind::Spec,
            ArtifactPayload::Diff => ArtifactKind::Diff,
            ArtifactPayload::Candidate(_) => ArtifactKind::Candidate,
            ArtifactPayload::TestReport(_) => ArtifactKind::TestReport,
            ArtifactPayload::Log => ArtifactKind::Log,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    /// Full commit id in the task's repository.
    pub base: String,
    /// Applied in index order.
    pub diffs: Vec<ArtId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunStatus {
    Passed,
    Failed,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileResult {
    pub name: String,
    pub exit_code: i32,
    pub failed: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub candidate: ArtId,
    pub run: RunId,
    pub profile_digest: Sha256,
    pub status: RunStatus,
    pub profiles: Vec<ProfileResult>,
    pub tree: Option<Sha256>,
    /// The sandbox profile the run executed under, as the supervisor names it
    /// (for example `landlock+seccomp:no-net`, or `unconfined` for a trusted
    /// local run). Reviewers read it; the core records it.
    pub sandbox: String,
    pub log: ArtId,
}

impl Report {
    /// R16: `passed` only if every profile has exit 0 and no failures.
    pub fn consistent(&self) -> bool {
        let clean = self
            .profiles
            .iter()
            .all(|p| p.exit_code == 0 && p.failed == 0);
        match self.status {
            RunStatus::Passed => clean && !self.profiles.is_empty(),
            RunStatus::Failed | RunStatus::Error => true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRecord {
    pub id: ArtId,
    pub author: PrincipalId,
    pub kind: ArtifactKind,
    pub blob: BlobRef,
    pub payload: ArtifactPayload,
    /// Stamped by the store on candidates: the approved spec at submission.
    pub spec: Option<ArtId>,
    pub candidate: Option<ArtId>,
    pub run: Option<RunId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MessageKind {
    Ask,
    Answer,
    Propose,
    Finding,
    Verdict,
    RequestDecision,
    Decision,
    Pass,
}

/// A reference to an artifact (with optional opaque fragment) or a message.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Ref {
    Art { id: ArtId, fragment: Option<String> },
    Msg(MsgId),
}

impl Ref {
    pub fn art(id: ArtId) -> Ref {
        Ref::Art { id, fragment: None }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Recipient {
    Role(Role),
    Human,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerdictKind {
    Approve,
    Revise,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockingItem {
    pub id: String,
    pub reference: Ref,
    pub issue: String,
    pub fix: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verdict {
    pub subject: ArtId,
    pub run: RunId,
    pub verdict: VerdictKind,
    pub blocking: Vec<BlockingItem>,
    pub non_blocking: Vec<BlockingItem>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Accept,
    Reject,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub subject: MsgId,
    pub outcome: Outcome,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Body {
    Prose(String),
    Verdict(Verdict),
    Decision(Decision),
}

/// What an author submits; the store assigns `id`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub kind: MessageKind,
    pub to: Vec<Recipient>,
    pub body: Body,
    pub refs: Vec<Ref>,
    pub evidence: Vec<Ref>,
    pub reply_to: Option<MsgId>,
    /// `pass` only.
    pub yield_to: Option<Role>,
    /// `request_decision` only: opens the plan gate.
    pub gate: bool,
}

impl Draft {
    pub fn new(kind: MessageKind, body: impl Into<String>) -> Draft {
        Draft {
            kind,
            to: vec![Recipient::All],
            body: Body::Prose(body.into()),
            refs: vec![],
            evidence: vec![],
            reply_to: None,
            yield_to: None,
            gate: false,
        }
    }
    pub fn to(mut self, r: Recipient) -> Draft {
        self.to = vec![r];
        self
    }
    pub fn refs(mut self, refs: Vec<Ref>) -> Draft {
        self.refs = refs;
        self
    }
    pub fn evidence(mut self, ev: Vec<Ref>) -> Draft {
        self.evidence = ev;
        self
    }
    pub fn reply_to(mut self, id: MsgId) -> Draft {
        self.reply_to = Some(id);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: MsgId,
    pub from: PrincipalId,
    pub from_role: Option<Role>,
    pub draft: Draft,
}

/// Protocol version carried on messages.
pub const PROTOCOL_VERSION: u32 = 4;
pub const BODY_CAP: usize = 1_200;
pub const FRAGMENT_CAP: usize = 128;
pub const READ_PAGE: usize = 20;

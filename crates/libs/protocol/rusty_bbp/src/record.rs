//! Records: principals, artifacts, messages and their typed payloads.

use crate::coedit::SpecEdit;
use crate::ids::*;
use rusty_serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrincipalKind {
    Human,
    Moderator,
    Runner,
    Agent { role: RoleId, vendor: String },
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
    /// Outside material for `cited` evidence (claims mode).
    Source,
}

/// Typed artifact contents, decoded by the adapter from the exact bytes stored (A1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactPayload {
    Brief,
    /// A spec must reference the task brief.
    Spec {
        brief: ArtId,
    },
    /// A co-edited spec version: line edits on a base version (step 4).
    SpecEdit(SpecEdit),
    Diff,
    Candidate(Candidate),
    TestReport(Report),
    Log,
    Source(Source),
}

/// Outside material cited as evidence. BBP checks shape and digest, not truth.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub origin: String,
    pub retrieved_at: String,
    pub excerpt: String,
    /// SHA-256 of `excerpt`.
    pub digest: Sha256,
}

pub const SOURCE_EXCERPT_CAP: usize = 4_000;

impl ArtifactKind {
    /// The kind's snake_case name, as the spec writes it.
    pub fn name(self) -> &'static str {
        match self {
            ArtifactKind::Brief => "brief",
            ArtifactKind::Spec => "spec",
            ArtifactKind::Diff => "diff",
            ArtifactKind::Candidate => "candidate",
            ArtifactKind::TestReport => "test_report",
            ArtifactKind::Log => "log",
            ArtifactKind::Source => "source",
        }
    }
}

impl ArtifactPayload {
    pub fn kind(&self) -> ArtifactKind {
        match self {
            ArtifactPayload::Brief => ArtifactKind::Brief,
            ArtifactPayload::Spec { .. } | ArtifactPayload::SpecEdit(_) => ArtifactKind::Spec,
            ArtifactPayload::Diff => ArtifactKind::Diff,
            ArtifactPayload::Candidate(_) => ArtifactKind::Candidate,
            ArtifactPayload::TestReport(_) => ArtifactKind::TestReport,
            ArtifactPayload::Log => ArtifactKind::Log,
            ArtifactPayload::Source(_) => ArtifactKind::Source,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    /// Full commit id in the task's repository.
    pub base: String,
    /// Applied in index order.
    pub diffs: Vec<ArtId>,
    /// Claims mode: the claim this candidate tests. Required there, refused in
    /// stage mode; absent, it is not written, so stage logs are unchanged.
    #[rusty_serde(default, skip_serializing_if = "Option::is_none")]
    pub claim: Option<MsgId>,
    /// Claims mode: which frozen profile set runs this candidate (#597).
    /// `build` is not written, so existing logs are unchanged.
    #[rusty_serde(default, skip_serializing_if = "Suite::is_build")]
    pub suite: Suite,
}

impl Candidate {
    /// Same base and diffs: the two manifests build the same code.
    pub fn same_code(&self, other: &Candidate) -> bool {
        self.base == other.base && self.diffs == other.diffs
    }
}

/// The profile set a claims-mode candidate runs under (#597). A holdout
/// candidate re-runs the current candidate's code against the holdout suite,
/// whose tests live outside every sandbox; it never becomes current.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Suite {
    #[default]
    Build,
    Holdout,
}

impl Suite {
    pub fn is_build(&self) -> bool {
        *self == Suite::Build
    }

    /// Profile names this suite may report: holdout profiles start with
    /// `holdout:` or `golden:`, build profiles never do, so a build run can
    /// neither set nor clear a holdout failure.
    pub fn owns_profile(self, name: &str) -> bool {
        let holdout = name.starts_with("holdout:") || name.starts_with("golden:");
        holdout == (self == Suite::Holdout)
    }
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
    /// Claims mode: files under the frozen `test_paths` that the candidate's
    /// diffs add, modify or delete (interim test-change rule). Not written empty.
    #[rusty_serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tests_changed: Vec<TestChange>,
}

/// One test file a candidate changes, as the runner reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestChange {
    pub path: String,
    pub change: FileChange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileChange {
    Added,
    Modified,
    Deleted,
}

impl TestChange {
    pub fn added(path: &str) -> TestChange {
        TestChange {
            path: path.into(),
            change: FileChange::Added,
        }
    }
    pub fn modified(path: &str) -> TestChange {
        TestChange {
            path: path.into(),
            change: FileChange::Modified,
        }
    }
    pub fn deleted(path: &str) -> TestChange {
        TestChange {
            path: path.into(),
            change: FileChange::Deleted,
        }
    }
}

impl Report {
    /// Profiles that failed: a non-zero exit or any failed test.
    pub fn failed_profiles(&self) -> Vec<String> {
        self.profiles
            .iter()
            .filter(|p| p.exit_code != 0 || p.failed > 0)
            .map(|p| p.name.clone())
            .collect()
    }

    pub fn listed_profiles(&self) -> Vec<String> {
        self.profiles.iter().map(|p| p.name.clone()).collect()
    }
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
    /// Claims mode: a proposition the board scores.
    Claim,
    /// Claims mode: support or counter for a claim.
    Evidence,
}

impl MessageKind {
    /// The kind's snake_case name, as the spec writes it.
    pub fn name(self) -> &'static str {
        match self {
            MessageKind::Ask => "ask",
            MessageKind::Answer => "answer",
            MessageKind::Propose => "propose",
            MessageKind::Finding => "finding",
            MessageKind::Verdict => "verdict",
            MessageKind::RequestDecision => "request_decision",
            MessageKind::Decision => "decision",
            MessageKind::Pass => "pass",
            MessageKind::Claim => "claim",
            MessageKind::Evidence => "evidence",
        }
    }
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
    Role(RoleId),
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
    Claim(crate::claims::ClaimBody),
    Evidence(crate::claims::EvidenceBody),
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
    pub yield_to: Option<RoleId>,
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
    pub from_role: Option<RoleId>,
    pub draft: Draft,
}

/// Protocol version carried on messages.
pub const PROTOCOL_VERSION: u32 = 5;
pub const BODY_CAP: usize = 1_200;
pub const FRAGMENT_CAP: usize = 128;
pub const READ_PAGE: usize = 20;

#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Driver boundary: a blob must already be stored, and the claimed payload
//! must be what the stored bytes decode to (plan amendments A1 and A3).
mod common;
use common::*;
use rusty_bbp::*;

#[test]
fn missing_blob_is_rejected_before_the_core_sees_it() {
    let mut fx = Fx::new();
    let rev = fx.rev();
    let ghost = BlobRef {
        sha: Sha256([9; 32]),
        len: 4,
    };
    assert_eq!(
        rejected(fx.put_raw(Role::Planner, ArtifactPayload::Diff, ghost)),
        Code::BlobMissing
    );
    assert_eq!(fx.rev(), rev, "a boundary rejection appends nothing");
}

#[test]
fn payload_must_match_the_stored_bytes() {
    let mut fx = Fx::new();
    fx.reach_build();
    let diff = stored(fx.put(Role::Coder, ArtifactPayload::Diff, b"--- a\n+++ b\n"));
    let a = Candidate {
        base: "a".repeat(40),
        diffs: vec![diff],
    };
    let b = Candidate {
        base: "b".repeat(40),
        diffs: vec![diff],
    };
    let bytes_of_a = encode_artifact(&ArtifactPayload::Candidate(a), b"").expect("encode");
    let blob = fx.d.store.blob_put(&bytes_of_a);
    assert_eq!(
        rejected(fx.put_raw(Role::Coder, ArtifactPayload::Candidate(b), blob)),
        Code::PayloadMismatch
    );
    assert_eq!(fx.st().state, State::Build, "nothing was submitted");
}

#[test]
fn spec_brief_reference_comes_from_the_file() {
    let mut fx = Fx::new();
    let brief = fx.st().brief.expect("brief");
    // Bytes name a different brief than the claimed payload.
    let file = SpecFile {
        brief: ArtId(42),
        markdown: "# Spec".into(),
    };
    let bytes = rusty_serde::json::to_string(&file)
        .expect("json")
        .into_bytes();
    let blob = fx.d.store.blob_put(&bytes);
    assert_eq!(
        rejected(fx.put_raw(Role::Planner, ArtifactPayload::Spec { brief }, blob)),
        Code::PayloadMismatch
    );
    // Honest bytes decode and pass through to the core's own checks.
    let honest = fx
        .d
        .store
        .blob_put(&encode_artifact(&ArtifactPayload::Spec { brief }, b"# Spec").expect("encode"));
    stored(fx.put_raw(Role::Planner, ArtifactPayload::Spec { brief }, honest));
}

#[test]
fn undecodable_bytes_and_wrong_length_are_rejected() {
    let mut fx = Fx::new();
    fx.reach_build();
    let garbage = fx.d.store.blob_put(b"not json");
    let c = Candidate {
        base: "a".repeat(40),
        diffs: vec![],
    };
    assert_eq!(
        rejected(fx.put_raw(Role::Coder, ArtifactPayload::Candidate(c), garbage)),
        Code::PayloadMismatch
    );
    let real = fx.d.store.blob_put(b"diff");
    let lied = BlobRef {
        sha: real.sha,
        len: real.len + 1,
    };
    assert_eq!(
        rejected(fx.put_raw(Role::Coder, ArtifactPayload::Diff, lied)),
        Code::PayloadMismatch
    );
}

#[test]
fn codec_round_trips_every_kind() {
    let brief = ArtId(1);
    let cases = vec![
        (ArtifactPayload::Brief, b"task brief".as_slice()),
        (ArtifactPayload::Diff, b"--- a".as_slice()),
        (ArtifactPayload::Log, b"ok".as_slice()),
        (ArtifactPayload::Spec { brief }, b"# md".as_slice()),
        (
            ArtifactPayload::Candidate(Candidate {
                base: "c".repeat(40),
                diffs: vec![ArtId(2), ArtId(3)],
            }),
            b"".as_slice(),
        ),
        (
            ArtifactPayload::TestReport(Report {
                candidate: ArtId(4),
                run: RunId(1),
                profile_digest: Sha256([7; 32]),
                status: RunStatus::Failed,
                profiles: vec![ProfileResult {
                    name: "t".into(),
                    exit_code: 1,
                    failed: 2,
                }],
                tree: None,
                sandbox: "test".into(),
                log: ArtId(5),
            }),
            b"".as_slice(),
        ),
    ];
    for (payload, body) in cases {
        let bytes = encode_artifact(&payload, body).expect("encode");
        assert_eq!(
            decode_artifact(payload.kind(), &bytes).expect("decode"),
            payload
        );
    }
}

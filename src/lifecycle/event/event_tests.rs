//! Focused proof for the typed run/repository journal event schema.

#![allow(dead_code, unused_imports)]

use super::{
    EVENT_VERSION, EventError, EventLog, EvidenceKind, EvidenceRef, INVOCATION_CHECKPOINT,
    JournalEvent, Operation, Outcome, RunStage,
};

fn run_id() -> String {
    "2026-08-13T16:00:00.000000000Z-a1b2c3d4e5f60718a1b2c3d4e5f60718".to_owned()
}

fn intent(checkpoint: u64) -> JournalEvent {
    JournalEvent::RunIntent {
        checkpoint,
        run_id: run_id(),
        stage: RunStage::Invocation,
    }
}

fn repository_intent(checkpoint: u64, attempt: u8) -> JournalEvent {
    JournalEvent::RepositoryIntent {
        checkpoint,
        run_id: run_id(),
        repository_id: "destination-a".to_owned(),
        operation: Operation::Synchronize,
        attempt,
    }
}

fn repository_result(checkpoint: u64, attempt: u8, outcome: Outcome) -> JournalEvent {
    JournalEvent::RepositoryResult {
        checkpoint,
        run_id: run_id(),
        repository_id: "destination-a".to_owned(),
        operation: Operation::Synchronize,
        attempt,
        outcome,
    }
}

fn terminal(checkpoint: u64, outcome: Outcome) -> JournalEvent {
    JournalEvent::Terminal {
        checkpoint,
        run_id: run_id(),
        outcome,
    }
}

fn record_ok(log: &mut EventLog, event: &JournalEvent) {
    log.record(event).expect("event must be accepted");
}

fn record_err(log: &mut EventLog, event: &JournalEvent, needle: &str) {
    let error = log.record(event).expect_err("event must be rejected");
    assert!(
        error.to_string().contains(needle),
        "error {error:?} must mention {needle:?}"
    );
}

#[test]
fn every_variant_renders_and_parses_round_trip() {
    let events = vec![
        intent(0),
        repository_intent(1, 1),
        repository_result(2, 1, Outcome::Success),
        JournalEvent::SnapshotRecorded {
            checkpoint: 3,
            run_id: run_id(),
            repository_id: "destination-a".to_owned(),
            snapshot_id: "snap-1".to_owned(),
            revision: "rev-abc".to_owned(),
        },
        JournalEvent::Evidence {
            checkpoint: 4,
            run_id: run_id(),
            repository_id: Some("destination-a".to_owned()),
            evidence: EvidenceRef::new(
                EvidenceKind::Git,
                ".omnirepo/runs/evidence/verify-1.log",
                128,
            )
            .expect("bounded evidence"),
            stage: None,
        },
        JournalEvent::Cancelled {
            checkpoint: 5,
            run_id: run_id(),
        },
        terminal(6, Outcome::Success),
    ];
    for event in &events {
        let rendered = event.render();
        let parsed = JournalEvent::parse(&rendered).expect("canonical line must parse");
        assert_eq!(&parsed, event, "round trip for {rendered:?}");
    }
}

#[test]
fn unknown_versions_and_types_fail() {
    let line = intent(0)
        .render()
        .replace(&format!("\"version\":{EVENT_VERSION}"), "\"version\":99");
    assert!(matches!(
        JournalEvent::parse(&line),
        Err(EventError::UnknownVersion(99))
    ));
    let line = intent(0)
        .render()
        .replace("\"type\":\"run_intent\"", "\"type\":\"nonsense\"");
    assert!(matches!(
        JournalEvent::parse(&line),
        Err(EventError::UnknownType(_))
    ));
    let line = intent(0)
        .render()
        .replace("\"status\":\"started\"", "\"status\":\"done\"");
    assert!(matches!(
        JournalEvent::parse(&line),
        Err(EventError::UnknownStatus(_))
    ));
}

#[test]
fn malformed_and_missing_fields_fail_closed() {
    let mut malformed_lines = vec!["not json\n".to_owned(), "{}\n".to_owned()];
    malformed_lines.push("{\"version\":1,\"checkpoint\":0}\n".to_owned());
    malformed_lines.push(intent(0).render().replace("\"run_id\"", "\"run id\""));
    for line in &malformed_lines {
        let error = JournalEvent::parse(line).expect_err("malformed line must fail");
        assert!(
            matches!(error, EventError::Malformed | EventError::MissingField(_)),
            "unexpected error for {line:?}: {error:?}"
        );
    }
}

#[test]
fn run_must_start_with_invocation_intent_and_only_once() {
    let mut log = EventLog::new();
    record_err(
        &mut log,
        &repository_intent(0, 1),
        "no run -> repository intent",
    );
    let wrong_stage = JournalEvent::RunIntent {
        checkpoint: 0,
        run_id: run_id(),
        stage: RunStage::Preflight,
    };
    record_err(&mut log, &wrong_stage, "non-invocation stage");
    record_ok(&mut log, &intent(INVOCATION_CHECKPOINT));
    record_err(&mut log, &intent(5), "duplicate run intent");
}

#[test]
fn checkpoints_are_monotonic_and_duplicates_and_regressions_fail() {
    let mut log = EventLog::new();
    record_ok(&mut log, &intent(0));
    record_err(
        &mut log,
        &repository_intent(0, 1),
        "checkpoint 0 is not after",
    );
    record_ok(&mut log, &repository_intent(1, 1));
    record_err(
        &mut log,
        &repository_result(1, 1, Outcome::Success),
        "checkpoint 1 is not after",
    );
    record_ok(&mut log, &repository_result(2, 1, Outcome::Success));
    // Gaps are allowed (a failed writer may skip), but they must stay ordered.
    record_ok(&mut log, &repository_intent(5, 2));
}

#[test]
fn repository_intent_requires_result_before_repeat_and_result_requires_intent() {
    let mut log = EventLog::new();
    record_ok(&mut log, &intent(0));
    record_ok(&mut log, &repository_intent(1, 1));
    record_err(
        &mut log,
        &repository_intent(2, 1),
        "duplicate intent without result",
    );
    record_err(
        &mut log,
        &repository_result(3, 2, Outcome::Success),
        "no matching intent",
    );
    let wrong_operation = JournalEvent::RepositoryResult {
        checkpoint: 3,
        run_id: run_id(),
        repository_id: "destination-a".to_owned(),
        operation: Operation::Verify,
        attempt: 1,
        outcome: Outcome::Success,
    };
    record_err(&mut log, &wrong_operation, "no matching intent");
    record_ok(&mut log, &repository_result(4, 1, Outcome::Success));
    record_ok(&mut log, &repository_intent(5, 1));
    record_ok(&mut log, &repository_result(6, 1, Outcome::Failed));
    record_ok(&mut log, &terminal(7, Outcome::Failed));
}

#[test]
fn terminal_and_cancelled_states_are_final() {
    let mut log = EventLog::new();
    record_ok(&mut log, &intent(0));
    record_ok(&mut log, &terminal(1, Outcome::Success));
    record_err(&mut log, &repository_intent(2, 1), "terminal run");
    record_err(&mut log, &terminal(3, Outcome::Failed), "already terminal");
    record_err(
        &mut log,
        &JournalEvent::Cancelled {
            checkpoint: 4,
            run_id: run_id(),
        },
        "already terminal",
    );

    let mut log = EventLog::new();
    record_ok(&mut log, &intent(0));
    record_ok(
        &mut log,
        &JournalEvent::Cancelled {
            checkpoint: 1,
            run_id: run_id(),
        },
    );
    record_err(&mut log, &repository_intent(2, 1), "terminal run");
    record_err(&mut log, &terminal(3, Outcome::Success), "already terminal");
    record_err(
        &mut log,
        &JournalEvent::Cancelled {
            checkpoint: 4,
            run_id: run_id(),
        },
        "already terminal",
    );
}

#[test]
fn terminal_cannot_carry_the_cancelled_outcome() {
    let mut log = EventLog::new();
    record_ok(&mut log, &intent(0));
    record_err(
        &mut log,
        &terminal(1, Outcome::Cancelled),
        "terminal -> cancelled outcome",
    );
}

#[test]
fn cancelled_repository_result_marks_the_run_terminal() {
    let mut log = EventLog::new();
    record_ok(&mut log, &intent(0));
    record_ok(&mut log, &repository_intent(1, 1));
    record_ok(&mut log, &repository_result(2, 1, Outcome::Cancelled));
    record_err(&mut log, &repository_intent(3, 2), "terminal run");
}

#[test]
fn snapshot_and_evidence_require_a_running_run() {
    let mut log = EventLog::new();
    record_err(
        &mut log,
        &JournalEvent::SnapshotRecorded {
            checkpoint: 0,
            run_id: run_id(),
            repository_id: "destination-a".to_owned(),
            snapshot_id: "snap-1".to_owned(),
            revision: "rev-1".to_owned(),
        },
        "no run",
    );
    record_ok(&mut log, &intent(0));
    record_ok(
        &mut log,
        &JournalEvent::SnapshotRecorded {
            checkpoint: 1,
            run_id: run_id(),
            repository_id: "destination-a".to_owned(),
            snapshot_id: "snap-1".to_owned(),
            revision: "rev-1".to_owned(),
        },
    );
    record_ok(
        &mut log,
        &JournalEvent::Evidence {
            checkpoint: 2,
            run_id: run_id(),
            repository_id: None,
            evidence: EvidenceRef::new(EvidenceKind::Process, "target/evidence/run.log", 32)
                .expect("bounded evidence"),
            stage: None,
        },
    );
    record_ok(&mut log, &terminal(3, Outcome::Success));
    record_err(
        &mut log,
        &JournalEvent::Evidence {
            checkpoint: 4,
            run_id: run_id(),
            repository_id: None,
            evidence: EvidenceRef::new(EvidenceKind::Process, "target/evidence/late.log", 32)
                .expect("bounded evidence"),
            stage: None,
        },
        "terminal run",
    );
}

#[test]
fn evidence_respects_bounds_and_redaction_policy() {
    let oversized_path = "x".repeat(super::MAX_EVIDENCE_PATH_BYTES + 1);
    assert!(matches!(
        EvidenceRef::new(EvidenceKind::Git, oversized_path, 1),
        Err(EventError::EvidenceBounds { .. })
    ));
    assert!(matches!(
        EvidenceRef::new(
            EvidenceKind::Git,
            "target/evidence/huge.log",
            super::MAX_EVIDENCE_BYTES + 1
        ),
        Err(EventError::EvidenceBounds { .. })
    ));
    for secret_path in [
        "target/evidence/api_token.txt",
        "logs/credential-dump.log",
        "run/secret.env",
        "keys/private_key.pem",
        "evidence/password.txt",
    ] {
        assert!(
            matches!(
                EvidenceRef::new(EvidenceKind::Agent, secret_path, 8),
                Err(EventError::SecretBearingEvidence { .. })
            ),
            "{secret_path:?} must be rejected"
        );
    }
    let ok = EvidenceRef::new(EvidenceKind::Agent, "target/evidence/verify-1.log", 8)
        .expect("clean evidence accepted");
    assert_eq!(ok.kind, EvidenceKind::Agent);
    assert_eq!(ok.bytes, 8);
}

#[test]
fn evidence_rejects_control_newline_ansi_and_json_metacharacters() {
    // Control/newline/ANSI injection must stay inert: the exact JSONL render
    // can never be extended through an evidence reference.
    for hostile in [
        "target/evidence/evil\n}\\u002c\"type\":\"terminal\"",
        "target/evidence\u{1b}[31mred.log",
        "target/evidence/tab\u{9}bed.log",
        "target/evidence/quote\"boom.log",
        "target/evidence/back\\slash.log",
        "target/evidence/del\u{7f}ete.log",
    ] {
        assert!(
            matches!(
                EvidenceRef::new(EvidenceKind::Git, hostile, 8),
                Err(EventError::UnsafeEvidencePath { .. })
            ),
            "hostile evidence path must be rejected: {hostile:?}"
        );
    }
}

#[test]
fn canonical_identities_are_reused_exactly_in_render_and_parse() {
    let id = "2026-08-13T16:00:00.000000000Z-deadbeefdeadbeefdeadbeefdeadbeef";
    let event = JournalEvent::RunIntent {
        checkpoint: 0,
        run_id: id.to_owned(),
        stage: RunStage::Invocation,
    };
    let rendered = event.render();
    assert!(rendered.contains(&format!("\"run_id\":\"{id}\"")));
    let parsed = JournalEvent::parse(&rendered).expect("parse");
    assert_eq!(parsed.run_id(), id);

    let repository_event = repository_intent(1, 1);
    let rendered = repository_event.render();
    assert!(rendered.contains("\"repository_id\":\"destination-a\""));
    assert!(rendered.contains("\"operation\":\"sync\""));
    assert!(rendered.contains("\"attempt\":1"));
}

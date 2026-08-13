//! Contract and adversarial tests for the structured test evidence boundary.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;

use serde_json::{Value, json};

use omnirepo_test_support::test_evidence::{
    ArtifactReference, ArtifactStore, DIAGNOSTIC_TRUNCATION_MARKER, DiagnosticRedactor,
    EVIDENCE_BUNDLE_SCHEMA, EventKind, EventRecorder, EvidenceBundle, EvidenceError,
    HARNESS_TERMINAL_MARKER, MAX_EVIDENCE_BYTES, Outcome, PeerAccounting, SourcePlanConfig,
    TEST_EVENT_SCHEMA, TerminalProjection, TestEvent, TestIdentity, execute_case,
    sanitize_channels,
};

fn identity(case_id: &str, suite: &str, attempt: u32) -> TestIdentity {
    TestIdentity::new(
        case_id,
        suite,
        "destination-a",
        "verification",
        SourcePlanConfig::new("source-a", "plan-a", "config-a").expect("identity policy"),
        attempt,
        90210,
        "component",
    )
    .expect("identity should be valid")
}

fn valid_jsonl() -> String {
    let recorder = EventRecorder::default();
    recorder
        .start(
            identity("parse-fixture", "adversarial", 1),
            ArtifactReference::none(),
        )
        .expect("start")
        .pass()
        .expect("terminal");
    recorder
        .finalize()
        .expect("bundle")
        .to_jsonl()
        .expect("JSONL")
}

fn valid_bundle() -> EvidenceBundle {
    EvidenceBundle::from_jsonl(&valid_jsonl()).expect("valid bundle")
}

fn two_peer_bundle() -> EvidenceBundle {
    let recorder = EventRecorder::default();
    for case_id in ["peer-a", "peer-b"] {
        recorder
            .start(
                identity(case_id, "adversarial", 1),
                ArtifactReference::none(),
            )
            .expect("start")
            .pass()
            .expect("terminal");
    }
    recorder.finalize().expect("bundle")
}

fn jsonl_values(contents: &str) -> Vec<Value> {
    contents
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON fixture"))
        .collect()
}

fn values_jsonl(values: &[Value]) -> String {
    values
        .iter()
        .map(|value| serde_json::to_string(value).expect("JSON fixture"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn assert_rejected(contents: &str) {
    assert!(
        EvidenceBundle::from_jsonl(contents).is_err(),
        "hostile evidence input was accepted:\n{contents}"
    );
}

#[test]
fn event_schema_repeats_all_stable_fields_on_start_and_terminal() {
    let recorder = EventRecorder::default();
    let mut guard = recorder
        .start(
            identity("case-fields", "suite-fields", 3),
            ArtifactReference::new("replays/case-fields.jsonl", "replay-fields")
                .expect("safe artifact"),
        )
        .expect("start event");
    guard
        .finish_with_duration(Outcome::Skipped, 17, Some("capability unavailable"))
        .expect("terminal event");

    let bundle = recorder.finalize().expect("bundle");
    assert_eq!(bundle.events.len(), 2);
    let [start, terminal] = bundle.events.as_slice() else {
        panic!("expected one start and one terminal");
    };
    for event in [start, terminal] {
        assert_eq!(event.schema, TEST_EVENT_SCHEMA);
        assert_eq!(event.identity.case_id, "case-fields");
        assert_eq!(event.identity.suite, "suite-fields");
        assert_eq!(event.identity.repository, "destination-a");
        assert_eq!(event.identity.stage, "verification");
        assert_eq!(event.identity.source_plan_config.source, "source-a");
        assert_eq!(event.identity.source_plan_config.plan, "plan-a");
        assert_eq!(event.identity.source_plan_config.config, "config-a");
        assert_eq!(event.identity.attempt, 3);
        assert_eq!(event.identity.seed, 90210);
        assert_eq!(event.identity.command, "component");
        assert_eq!(
            event.artifact.path.as_deref(),
            Some("replays/case-fields.jsonl")
        );
        assert_eq!(event.artifact.replay_id.as_deref(), Some("replay-fields"));
    }
    assert_eq!(start.event_kind, EventKind::Start);
    assert!(!start.terminal);
    assert_eq!(start.outcome, Outcome::Started);
    assert_eq!(terminal.event_kind, EventKind::Terminal);
    assert!(terminal.terminal);
    assert_eq!(terminal.outcome, Outcome::Skipped);
    assert_eq!(terminal.duration_ms, 17);
}

#[test]
fn skip_and_harness_failure_are_terminalized_without_losing_peers() {
    let recorder = EventRecorder::default();
    let skip = recorder
        .start(
            identity("capability-skip", "suite", 1),
            ArtifactReference::none(),
        )
        .expect("skip start");
    skip.skip("unix capability is unavailable")
        .expect("skip terminal");

    let dropped = recorder
        .start(
            identity("worker-dropped", "suite", 1),
            ArtifactReference::none(),
        )
        .expect("dropped start");
    drop(dropped);

    let bundle = recorder.finalize().expect("bundle");
    assert_eq!(
        bundle.peer_accounting.expected_case_ids,
        ["capability-skip", "worker-dropped"]
    );
    assert_eq!(
        bundle.peer_accounting.terminal_case_ids,
        ["capability-skip", "worker-dropped"]
    );
    assert!(bundle.peer_accounting.missing_case_ids.is_empty());
    assert_eq!(bundle.projection.skipped, 1);
    assert_eq!(bundle.projection.harness_failures, 1);
    assert_eq!(bundle.projection.outcome, Outcome::HarnessFailure);
    let harness = bundle
        .events
        .iter()
        .find(|event| event.identity.case_id == "worker-dropped" && event.terminal)
        .expect("synthetic harness terminal");
    assert_eq!(harness.diagnostic.as_deref(), Some(HARNESS_TERMINAL_MARKER));
}

#[test]
fn parallel_event_jsonl_is_byte_deterministic_across_completion_orders() {
    fn run(order: &[&str]) -> String {
        let recorder = EventRecorder::default();
        let handles = order
            .iter()
            .map(|case_id| {
                let recorder = recorder.clone();
                let identity = identity(case_id, "parallel", 1);
                thread::spawn(move || {
                    let mut guard = recorder
                        .start(identity, ArtifactReference::none())
                        .expect("parallel start");
                    guard
                        .finish_with_duration(Outcome::Passed, 11, Some("peer diagnostic"))
                        .expect("parallel terminal");
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().expect("worker");
        }
        recorder
            .finalize()
            .expect("parallel bundle")
            .to_jsonl()
            .expect("JSONL")
    }

    let first = run(&["case-z", "case-a", "case-m"]);
    let second = run(&["case-m", "case-z", "case-a"]);
    assert_eq!(first, second);
    let lines = first.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 7);
    assert!(
        lines
            .windows(2)
            .all(|pair| !pair[0].is_empty() && !pair[1].is_empty())
    );
}

#[test]
fn redaction_removes_known_values_credentials_uri_userinfo_and_controls() {
    let redactor = DiagnosticRedactor::new(["known-secret", "short", "short"]);
    let input = concat!(
        "Authorization: Bearer known-secret\n",
        concat!("pass", "word='pw-value' tok", "en=token-value "),
        concat!("https://user:pass", "word@example.test/x "),
        "\x1b[2Ksecret\u{0007}"
    );
    let diagnostic = redactor.sanitize(input);
    assert!(diagnostic.redacted);
    assert!(!diagnostic.text.contains("known-secret"));
    assert!(!diagnostic.text.contains("pw-value"));
    assert!(!diagnostic.text.contains("token-value"));
    assert!(!diagnostic.text.contains("user:password"));
    assert!(!diagnostic.text.contains('\x1b'));
    assert!(!diagnostic.text.contains('\u{0007}'));
    assert!(
        diagnostic.text.contains("[control-sequence]"),
        "sanitized diagnostic: {:?}",
        diagnostic.text
    );
}

#[test]
fn combined_channels_use_one_bound_and_one_canonical_marker() {
    let redactor = DiagnosticRedactor::new(["known-secret"]);
    let channels = sanitize_channels(
        &redactor,
        b"stdout known-secret ",
        b"stderr payload that exceeds the shared channel budget",
        64,
    )
    .expect("valid channel bound");

    assert_eq!(
        channels.combined_bytes,
        channels.stdout.text.len() + channels.stderr.text.len()
    );
    assert!(channels.combined_bytes <= 64);
    let combined = format!("{}{}", channels.stdout.text, channels.stderr.text);
    assert_eq!(combined.matches(DIAGNOSTIC_TRUNCATION_MARKER).count(), 1);
    assert!(!combined.contains("known-secret"));
    assert!(channels.stdout.redacted);
    assert!(channels.stderr.truncated);
}

#[test]
fn combined_channels_decode_non_utf8_and_escape_controls_before_bounding() {
    let redactor = DiagnosticRedactor::default();
    let channels = sanitize_channels(&redactor, b"prefix\xff\x1b[31m", b"suffix\x07", 128)
        .expect("valid channel bound");

    assert!(channels.stdout.non_utf8);
    assert!(channels.stdout.control_escaped);
    assert!(channels.stderr.control_escaped);
    assert!(!channels.stdout.text.contains('\x1b'));
    assert!(!channels.stderr.text.contains('\u{0007}'));
    assert!(channels.stdout.text.contains("[control-sequence]"));
    assert!(channels.combined_bytes <= 128);
}

#[test]
fn combined_channels_reject_unusable_bounds() {
    let redactor = DiagnosticRedactor::default();
    assert!(sanitize_channels(&redactor, b"out", b"err", 0).is_err());
    assert!(sanitize_channels(&redactor, b"out", b"err", MAX_EVIDENCE_BYTES + 1).is_err());
}

#[test]
fn combined_diagnostics_never_exceed_one_mib_and_keep_a_truncation_marker() {
    let recorder = EventRecorder::new(DiagnosticRedactor::default());
    for index in 0..8 {
        let guard = recorder
            .start(
                identity(&format!("large-{index}"), "bounds", 1),
                ArtifactReference::none(),
            )
            .expect("large start");
        guard
            .fail("diagnostic".repeat(MAX_EVIDENCE_BYTES / 4))
            .expect("large terminal");
    }
    let bundle = recorder.finalize().expect("bounded bundle");
    let diagnostic_bytes = bundle
        .events
        .iter()
        .filter_map(|event| event.diagnostic.as_ref())
        .map(String::len)
        .sum::<usize>();
    assert!(diagnostic_bytes <= MAX_EVIDENCE_BYTES);
    assert!(bundle.events.iter().any(|event| {
        event
            .diagnostic
            .as_deref()
            .is_some_and(|text| text.contains(DIAGNOSTIC_TRUNCATION_MARKER))
    }));
}

#[test]
fn malformed_or_tampered_jsonl_fails_closed() {
    let recorder = EventRecorder::default();
    recorder
        .start(
            identity("parse", "adversarial", 1),
            ArtifactReference::none(),
        )
        .expect("start")
        .pass()
        .expect("terminal");
    let bundle = recorder.finalize().expect("bundle");
    let jsonl = bundle.to_jsonl().expect("JSONL");
    assert!(matches!(
        EvidenceBundle::from_jsonl(&jsonl.replace(EVIDENCE_BUNDLE_SCHEMA, "old.schema")),
        Err(EvidenceError::InvalidField {
            field: "schema",
            ..
        })
    ));
    assert!(matches!(
        EvidenceBundle::from_jsonl("not-json\n"),
        Err(EvidenceError::Json(_))
    ));
    let duplicate = format!(
        "{}\n{}",
        serde_json::to_string(&bundle.events[0]).expect("start JSON"),
        serde_json::to_string(&bundle.events[0]).expect("duplicate JSON")
    );
    assert!(EvidenceBundle::from_jsonl(&duplicate).is_err());
}

#[test]
fn safe_artifact_store_rejects_absolute_parent_and_symlink_paths() {
    let root = tempfile::tempdir().expect("artifact root");
    let store = ArtifactStore::new(root.path()).expect("store");
    assert!(store.resolve("/tmp/outside").is_err());
    assert!(store.resolve("../outside").is_err());
    assert!(store.resolve("a/../../outside").is_err());

    let pointer = store
        .write_bytes("replays/case.jsonl", b"{\"ok\":true}\n")
        .expect("safe artifact");
    assert_eq!(pointer.path.as_deref(), Some("replays/case.jsonl"));
    assert!(pointer.replay_id.is_some());

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.path().join("outside"), root.path().join("link"))
            .expect("symlink fixture");
        assert!(store.resolve("link/escape.jsonl").is_err());
        assert!(store.write_bytes("link/escape.jsonl", b"leak").is_err());
    }
}

#[test]
fn cleanup_is_run_after_body_failure_and_only_structured_terminal_is_emitted() {
    let recorder = EventRecorder::default();
    let cleanup_ran = Arc::new(AtomicBool::new(false));
    let cleanup_ran_by_closure = Arc::clone(&cleanup_ran);
    let execution = execute_case(
        &recorder,
        identity("cleanup", "lifecycle", 1),
        ArtifactReference::none(),
        || Err("assertion failed".to_owned()),
        move || {
            cleanup_ran_by_closure.store(true, Ordering::SeqCst);
            Ok(())
        },
    )
    .expect("case execution");
    assert!(cleanup_ran.load(Ordering::SeqCst));
    assert_eq!(execution.outcome, Outcome::Failed);
    let bundle = recorder.finalize().expect("bundle");
    assert_eq!(
        bundle
            .events
            .iter()
            .filter(|event| event.event_kind == EventKind::Terminal)
            .count(),
        1
    );
    assert!(
        !bundle
            .projection
            .render_quiet()
            .contains("assertion failed")
    );
}

#[test]
fn cleanup_failure_becomes_harness_failure_after_body_success() {
    let recorder = EventRecorder::default();
    let execution = execute_case(
        &recorder,
        identity("cleanup-failure", "lifecycle", 1),
        ArtifactReference::none(),
        || Ok(()),
        || Err("cleanup could not remove fixture".to_owned()),
    )
    .expect("case execution");
    assert_eq!(execution.outcome, Outcome::HarnessFailure);
    let bundle = recorder.finalize().expect("bundle");
    let terminal = bundle.events.iter().find(|event| event.terminal).unwrap();
    assert_eq!(terminal.outcome, Outcome::HarnessFailure);
    assert!(
        terminal
            .diagnostic
            .as_deref()
            .is_some_and(|value| value.contains("cleanup could not remove fixture"))
    );
}

#[test]
fn event_recording_is_replay_stable_for_many_generated_case_ids() {
    let recorder = EventRecorder::default();
    for index in 0..128 {
        let case_id = format!("generated-case-{index:03}");
        let guard = recorder
            .start(
                identity(&case_id, "property", index % 3 + 1),
                ArtifactReference::none(),
            )
            .expect("generated start");
        guard.pass().expect("generated terminal");
    }
    let bundle = recorder.finalize().expect("generated bundle");
    let jsonl = bundle.to_jsonl().expect("generated JSONL");
    let replay = EvidenceBundle::from_jsonl(&jsonl).expect("replay JSONL");
    assert_eq!(replay, bundle);
    assert_eq!(bundle.peer_accounting.expected_case_ids.len(), 128);
    assert!(
        bundle
            .peer_accounting
            .terminal_outcomes
            .values()
            .all(|outcome| *outcome == Outcome::Passed)
    );
}

#[test]
fn jsonl_rejects_leading_trailing_and_interior_blank_records() {
    let valid = valid_jsonl();
    let lines = valid.lines().collect::<Vec<_>>();

    assert_rejected(&format!("\n{valid}"));
    assert_rejected(&format!("{valid}\n"));
    assert_rejected(&format!("{}\n\n{}\n", lines[0], lines[1]));
}

#[test]
fn jsonl_rejects_unknown_keys_at_each_object_level() {
    let valid = valid_jsonl();
    let original = jsonl_values(&valid);

    let mut event = original.clone();
    event[0]["unexpected_event_key"] = json!(true);
    assert_rejected(&values_jsonl(&event));

    let mut artifact = original.clone();
    artifact[0]["artifact"]["unexpected_artifact_key"] = json!(true);
    assert_rejected(&values_jsonl(&artifact));

    let mut summary = original.clone();
    summary[2]["unexpected_summary_key"] = json!(true);
    assert_rejected(&values_jsonl(&summary));

    let mut accounting = original.clone();
    accounting[2]["peer_accounting"]["unexpected_accounting_key"] = json!(true);
    assert_rejected(&values_jsonl(&accounting));

    let mut projection = original;
    projection[2]["projection"]["unexpected_projection_key"] = json!(true);
    assert_rejected(&values_jsonl(&projection));
}

#[test]
fn jsonl_requires_exactly_one_final_summary_after_at_least_one_event() {
    let valid = valid_jsonl();
    let original = jsonl_values(&valid);

    let summary_only = values_jsonl(&[original[2].clone()]);
    assert_rejected(&summary_only);

    let duplicate_summary = values_jsonl(&[
        original[0].clone(),
        original[2].clone(),
        original[2].clone(),
    ]);
    assert_rejected(&duplicate_summary);

    let nonfinal_summary = values_jsonl(&[
        original[2].clone(),
        original[0].clone(),
        original[1].clone(),
    ]);
    assert_rejected(&nonfinal_summary);
}

#[test]
fn jsonl_rejects_tampered_counts_projection_accounting_and_event_order() {
    let valid = valid_jsonl();

    let mut counts = jsonl_values(&valid);
    counts[2]["projection"]["passed"] = json!(9);
    assert_rejected(&values_jsonl(&counts));

    let mut projection = jsonl_values(&valid);
    projection[2]["projection"]["artifact_path"] = json!("tampered/path");
    assert_rejected(&values_jsonl(&projection));

    let mut accounting = jsonl_values(&valid);
    accounting[2]["peer_accounting"]["expected_case_ids"] = json!([]);
    assert_rejected(&values_jsonl(&accounting));

    let mut outcomes = jsonl_values(&valid);
    outcomes[2]["peer_accounting"]["terminal_outcomes"] = json!({
        "wrong-correlation": "failed"
    });
    assert_rejected(&values_jsonl(&outcomes));

    let mut outcome = jsonl_values(&valid);
    outcome[1]["outcome"] = json!("failed");
    assert_rejected(&values_jsonl(&outcome));

    let mut reversed = jsonl_values(&valid);
    reversed.swap(0, 1);
    assert_rejected(&values_jsonl(&reversed));
}

#[test]
fn jsonl_rejects_tampered_event_ids_correlations_identity_and_schema() {
    let valid = valid_jsonl();

    let mut event_id = jsonl_values(&valid);
    event_id[0]["event_id"] = json!("not-the-correlated-start");
    assert_rejected(&values_jsonl(&event_id));

    let mut correlation = jsonl_values(&valid);
    correlation[0]["correlation_id"] = json!("not-the-correlated-id");
    assert_rejected(&values_jsonl(&correlation));

    let mut identity = jsonl_values(&valid);
    identity[0]["case_id"] = json!("tampered-case");
    assert_rejected(&values_jsonl(&identity));

    let mut schema = jsonl_values(&valid);
    schema[0]["schema"] = json!("unknown.event.schema");
    assert_rejected(&values_jsonl(&schema));
}

#[test]
fn jsonl_rejects_started_terminal_and_missing_or_duplicate_pairs() {
    let valid = valid_jsonl();

    let mut started_terminal = jsonl_values(&valid);
    started_terminal[1]["outcome"] = json!("started");
    assert_rejected(&values_jsonl(&started_terminal));

    let original = jsonl_values(&valid);
    assert_rejected(&values_jsonl(&[original[0].clone(), original[2].clone()]));
    assert_rejected(&values_jsonl(&[original[1].clone(), original[2].clone()]));
    assert_rejected(&values_jsonl(&[
        original[0].clone(),
        original[0].clone(),
        original[2].clone(),
    ]));
    assert_rejected(&values_jsonl(&[
        original[0].clone(),
        original[1].clone(),
        original[1].clone(),
        original[2].clone(),
    ]));
}

#[test]
fn jsonl_rejects_unsafe_deserialized_artifact_replay_and_identity_fields() {
    let valid = valid_jsonl();

    let mut artifact = jsonl_values(&valid);
    artifact[0]["artifact"]["path"] = json!("../escape");
    assert_rejected(&values_jsonl(&artifact));

    let mut replay = jsonl_values(&valid);
    replay[0]["artifact"]["replay_id"] = json!("replay\ncontrol");
    assert_rejected(&values_jsonl(&replay));

    let mut identity = jsonl_values(&valid);
    identity[0]["case_id"] = json!("case\ncontrol");
    assert_rejected(&values_jsonl(&identity));
}

#[test]
fn public_deserialization_rechecks_identity_and_artifact_constructors() {
    let identity_value =
        serde_json::to_value(identity("deserialize", "strict", 1)).expect("identity JSON");
    let mut empty_identity = identity_value.clone();
    empty_identity["case_id"] = json!("");
    assert!(serde_json::from_value::<TestIdentity>(empty_identity).is_err());

    let mut control_identity = identity_value;
    control_identity["case_id"] = json!("case\ncontrol");
    assert!(serde_json::from_value::<TestIdentity>(control_identity).is_err());

    let mut unsafe_artifact = json!({
        "path": "../escape",
        "replay_id": "replay-id"
    });
    assert!(serde_json::from_value::<ArtifactReference>(unsafe_artifact).is_err());

    unsafe_artifact = json!({
        "path": "safe/path",
        "replay_id": "replay\ncontrol"
    });
    assert!(serde_json::from_value::<ArtifactReference>(unsafe_artifact).is_err());

    unsafe_artifact = json!({
        "path": "safe/path",
        "replay_id": "../replay"
    });
    assert!(serde_json::from_value::<ArtifactReference>(unsafe_artifact).is_err());
}

#[test]
fn public_deserialization_rechecks_accounting_projection_and_artifact_invariants() {
    let bundle = valid_bundle();

    let mut started_outcome = serde_json::to_value(&bundle.peer_accounting).expect("accounting");
    let correlation = bundle
        .peer_accounting
        .terminal_outcomes
        .keys()
        .next()
        .expect("terminal correlation")
        .clone();
    started_outcome["terminal_outcomes"][correlation] = json!("started");
    assert!(serde_json::from_value::<PeerAccounting>(started_outcome).is_err());

    let mut duplicate_expected = serde_json::to_value(&bundle.peer_accounting).expect("accounting");
    duplicate_expected["expected_case_ids"] = json!(["parse-fixture", "parse-fixture"]);
    assert!(serde_json::from_value::<PeerAccounting>(duplicate_expected).is_err());

    let mut inconsistent_partition =
        serde_json::to_value(&bundle.peer_accounting).expect("accounting");
    inconsistent_partition["terminal_case_ids"] = json!([]);
    assert!(serde_json::from_value::<PeerAccounting>(inconsistent_partition).is_err());

    let mut inconsistent_projection = serde_json::to_value(&bundle.projection).expect("projection");
    inconsistent_projection["failed"] = json!(1);
    assert!(serde_json::from_value::<TerminalProjection>(inconsistent_projection).is_err());

    let mut changed_artifact = jsonl_values(&bundle.to_jsonl().expect("JSONL"));
    changed_artifact[1]["artifact"] = json!({
        "path": "replays/changed.jsonl",
        "replay_id": "replay-changed"
    });
    assert_rejected(&values_jsonl(&changed_artifact));
}

#[test]
fn public_event_and_bundle_deserialization_reject_unbounded_persisted_diagnostics() {
    let bundle = valid_bundle();
    let oversized = "x".repeat(MAX_EVIDENCE_BYTES + 1);

    let mut event = serde_json::to_value(&bundle.events[1]).expect("event");
    event["diagnostic"] = json!(oversized.clone());
    assert!(serde_json::from_value::<TestEvent>(event).is_err());

    let mut bundle_value = serde_json::to_value(&bundle).expect("bundle");
    bundle_value["events"][1]["diagnostic"] = json!(oversized);
    assert!(serde_json::from_value::<EvidenceBundle>(bundle_value).is_err());

    let mut jsonl = jsonl_values(&bundle.to_jsonl().expect("JSONL"));
    jsonl[1]["diagnostic"] = json!("x".repeat(MAX_EVIDENCE_BYTES + 1));
    assert_rejected(&values_jsonl(&jsonl));
}

#[test]
fn public_bundle_deserialization_rejects_combined_diagnostic_budget() {
    let bundle = two_peer_bundle();
    let per_event = "x".repeat(MAX_EVIDENCE_BYTES / 2 + 1);

    let mut bundle_value = serde_json::to_value(&bundle).expect("bundle");
    bundle_value["events"][1]["diagnostic"] = json!(per_event.clone());
    bundle_value["events"][3]["diagnostic"] = json!(per_event);
    assert!(serde_json::from_value::<TestEvent>(bundle_value["events"][1].clone()).is_ok());
    assert!(serde_json::from_value::<TestEvent>(bundle_value["events"][3].clone()).is_ok());
    assert!(serde_json::from_value::<EvidenceBundle>(bundle_value).is_err());

    let mut jsonl = jsonl_values(&bundle.to_jsonl().expect("JSONL"));
    jsonl[1]["diagnostic"] = json!("x".repeat(MAX_EVIDENCE_BYTES / 2 + 1));
    jsonl[3]["diagnostic"] = json!("x".repeat(MAX_EVIDENCE_BYTES / 2 + 1));
    assert_rejected(&values_jsonl(&jsonl));
}

#[test]
fn public_debug_redacts_secrets_and_body_or_cleanup_diagnostics() {
    let redactor = DiagnosticRedactor::new(["debug-secret"]);
    let redactor_debug = format!("{redactor:?}");
    assert!(!redactor_debug.contains("debug-secret"));

    let recorder = EventRecorder::default();
    let execution = execute_case(
        &recorder,
        identity("debug-case", "debug", 1),
        ArtifactReference::none(),
        || Err("raw-body-diagnostic".to_owned()),
        || Err("raw-cleanup-diagnostic".to_owned()),
    )
    .expect("case execution");
    let execution_debug = format!("{execution:?}");
    assert!(!execution_debug.contains("raw-body-diagnostic"));
    assert!(!execution_debug.contains("raw-cleanup-diagnostic"));

    let bundle_debug = format!("{:?}", recorder.finalize().expect("bundle"));
    assert!(!bundle_debug.contains("raw-body-diagnostic"));
    assert!(!bundle_debug.contains("raw-cleanup-diagnostic"));
}

#[test]
fn artifact_store_rejects_symlink_ancestors_and_non_directory_components() {
    let parent = tempfile::tempdir().expect("parent");
    let real_parent = tempfile::tempdir().expect("real parent");

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(real_parent.path(), parent.path().join("alias"))
            .expect("ancestor symlink fixture");
        assert!(ArtifactStore::new(parent.path().join("alias/artifacts")).is_err());

        let root_target = tempfile::tempdir().expect("root target");
        let root_alias = parent.path().join("root-alias");
        std::os::unix::fs::symlink(root_target.path(), &root_alias).expect("root symlink fixture");
        assert!(ArtifactStore::new(root_alias).is_err());
    }

    let file_root = parent.path().join("file-root");
    std::fs::write(&file_root, b"file").expect("file root fixture");
    assert!(ArtifactStore::new(file_root.join("nested")).is_err());

    let store = ArtifactStore::new(parent.path().join("artifacts")).expect("store");
    std::fs::write(store.root().join("not-a-directory"), b"file").expect("file fixture");
    assert!(store.resolve("not-a-directory/child.jsonl").is_err());
    assert!(
        store
            .write_bytes("not-a-directory/child.jsonl", b"escape")
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn artifact_store_does_not_follow_an_ancestor_swapped_to_a_symlink() {
    let root = tempfile::tempdir().expect("artifact root");
    let outside = tempfile::tempdir().expect("outside root");
    let store = ArtifactStore::new(root.path()).expect("store");
    std::fs::create_dir(root.path().join("swappable")).expect("child directory");
    std::fs::rename(root.path().join("swappable"), root.path().join("moved")).expect("move child");
    std::os::unix::fs::symlink(outside.path(), root.path().join("swappable"))
        .expect("swapped symlink");

    assert!(
        store
            .write_bytes("swappable/escape.jsonl", b"no leak")
            .is_err()
    );
    assert!(!outside.path().join("escape.jsonl").exists());
}

#[cfg(unix)]
#[test]
fn artifact_store_does_not_follow_a_root_ancestor_swapped_to_a_symlink() {
    let parent = tempfile::tempdir().expect("parent");
    let outside = tempfile::tempdir().expect("outside root");
    let root_parent = parent.path().join("authority");
    let root = root_parent.join("artifacts");
    std::fs::create_dir_all(&root).expect("authority root");
    std::fs::create_dir(outside.path().join("artifacts")).expect("outside artifacts");
    let store = ArtifactStore::new(&root).expect("store");

    std::fs::rename(&root_parent, parent.path().join("moved-authority")).expect("move root");
    std::os::unix::fs::symlink(outside.path(), &root_parent).expect("swapped root ancestor");

    assert!(store.write_bytes("escape.jsonl", b"no leak").is_err());
    assert!(!outside.path().join("artifacts/escape.jsonl").exists());
}

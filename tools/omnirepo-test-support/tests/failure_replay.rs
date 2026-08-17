//! Focused contract tests for deterministic failure replay bundles.

use std::{collections::BTreeMap, fs};

use omnirepo_test_support::failure_replay::{
    AssertionFailure, BarrierAction, BarrierStep, CapabilitySnapshot, CleanupFailure, DurableEvent,
    EffectDiff, EffectDifference, FailureClass, FailureReplayBundle, FailureReplayError,
    FailureReplaySpec, FailureScenario, NonReplayableReason, Outcome, PeerOutcome,
    PlatformContract, ReplayCommand, ReplayDivergence, ReplayObservation, ReplayVerification,
};
use omnirepo_test_support::test_evidence::{
    ArtifactReference, DiagnosticRedactor, EventRecorder, SourcePlanConfig, TestIdentity,
};
use omnirepo_test_support::{
    agent_double::AgentDouble,
    cross_domain_fixture::{
        ContentDecisionFixture, ContentFixture, ContentModeFixture, CrossDomainFixture,
        FixtureContext, FixtureSpec as DomainFixtureSpec, MarkerTopologyFixture,
    },
    git_double::LocalGitRemoteDouble,
    lifecycle_fixture::{FixtureOutcome, FixtureSpec, LifecycleFixture},
    recovery_control::{
        ConcurrentRunControl, CrashSpec, CrashableParent, JournalControl, JournalTail,
        RetainedState,
    },
};

fn evidence(
    case_id: &str,
    outcome: Outcome,
) -> omnirepo_test_support::test_evidence::EvidenceBundle {
    let recorder = EventRecorder::default();
    let identity = TestIdentity::new(
        case_id,
        "failure-replay",
        "destination",
        "verification",
        SourcePlanConfig::new("source", "plan", "config").expect("identity source"),
        1,
        7405,
        "component",
    )
    .expect("identity");
    let mut step = recorder
        .start(identity, ArtifactReference::none())
        .expect("start");
    step.finish_with_duration(outcome, 17, Some("assertion diagnostic"))
        .expect("terminal");
    recorder.finalize().expect("evidence")
}

fn chunked_evidence() -> omnirepo_test_support::test_evidence::EvidenceBundle {
    let recorder = EventRecorder::default();
    for (case_id, outcome, diagnostic) in [
        ("chunk-a", Outcome::Failed, "known-"),
        ("chunk-b", Outcome::Passed, "secret\u{1b}[31mred"),
    ] {
        let identity = TestIdentity::new(
            case_id,
            "failure-replay",
            "destination",
            "verification",
            SourcePlanConfig::new("source", "plan", "config").expect("identity source"),
            1,
            74_505,
            "component",
        )
        .expect("identity");
        let mut step = recorder
            .start(identity, ArtifactReference::none())
            .expect("start");
        step.finish_with_duration(outcome, 17, Some(diagnostic))
            .expect("terminal");
    }
    recorder.finalize().expect("evidence")
}

fn spec() -> FailureReplaySpec {
    spec_with_evidence(evidence("crash-case", Outcome::Failed))
}

fn spec_with_evidence(
    evidence: omnirepo_test_support::test_evidence::EvidenceBundle,
) -> FailureReplaySpec {
    let mut config = BTreeMap::new();
    config.insert("mode".to_owned(), "offline".to_owned());
    config.insert("credential".to_owned(), "known-secret".to_owned());
    FailureReplaySpec::new(
        "test-manifest/v3",
        "crash-case",
        "fixture-crash-7405",
        FailureScenario::ProcessCrash,
        FailureClass::ProductFailure,
        7405,
        evidence,
    )
    .platform(PlatformContract::new(
        "linux-ext4",
        vec![
            CapabilitySnapshot::available("process-tree"),
            CapabilitySnapshot::unsupported("pty", "not selected"),
        ],
    ))
    .command(CommandSummaryFixture::summary(config))
    .event_log("events/crash-case.jsonl")
    .barriers(vec![
        BarrierStep::new(1, "journal.after-flush", BarrierAction::Hit),
        BarrierStep::new(2, "journal.after-flush", BarrierAction::Released),
    ])
    .first_failure(AssertionFailure::new(
        3,
        "assertion.git.head",
        "old-head",
        "new-head",
    ))
    .durable_events(vec![
        DurableEvent::new(1, "journal.append", "stage=intent"),
        DurableEvent::new(2, "journal.flush", "stage=durable"),
    ])
    .effect_diff(EffectDiff::new(vec![EffectDifference::new(
        "destination/.managed",
        Some("old"),
        Some("new"),
    )]))
    .peers(vec![PeerOutcome::new("peer-a", Outcome::Passed, None)])
    .cleanup_failures(vec![CleanupFailure::new(
        "fixture.cleanup",
        "cleanup token=known-secret",
    )])
    .replayable(ReplayCommand::new(
        "cargo",
        [
            "test",
            "--package",
            "omnirepo-test-support",
            "--",
            "--exact",
            "crash-case",
        ],
    ))
}

struct CommandSummaryFixture;

impl CommandSummaryFixture {
    fn summary(
        config: BTreeMap<String, String>,
    ) -> omnirepo_test_support::failure_replay::CommandSummary {
        omnirepo_test_support::failure_replay::CommandSummary::new(
            "cargo",
            ["test", "--offline", "--seed", "7405"],
            config,
        )
    }
}

#[derive(Clone)]
struct ScenarioFacts {
    case_id: String,
    fixture_id: String,
    scenario: FailureScenario,
    failure_class: FailureClass,
    outcome: Outcome,
    seed: u64,
    platform: PlatformContract,
    barriers: Vec<BarrierStep>,
    first_failure: AssertionFailure,
    durable_events: Vec<DurableEvent>,
    effect_diff: EffectDiff,
}

struct ScenarioDetails {
    scenario: FailureScenario,
    failure_class: FailureClass,
    outcome: Outcome,
    seed: u64,
    barriers: Vec<BarrierStep>,
    first_failure: AssertionFailure,
    durable_events: Vec<DurableEvent>,
    effect_diff: EffectDiff,
}

fn scenario_platform() -> PlatformContract {
    PlatformContract::new(
        "linux-ext4",
        vec![
            CapabilitySnapshot::available("process-tree"),
            CapabilitySnapshot::available("local-hermetic-controls"),
        ],
    )
}

fn scenario_facts(case_id: &str, details: ScenarioDetails) -> ScenarioFacts {
    let fixture = CrossDomainFixture::new(
        DomainFixtureSpec::new(case_id, details.seed).expect("scenario fixture identity"),
    );
    ScenarioFacts {
        case_id: case_id.to_owned(),
        fixture_id: fixture.identity().fixture_id().to_owned(),
        scenario: details.scenario,
        failure_class: details.failure_class,
        outcome: details.outcome,
        seed: details.seed,
        platform: scenario_platform(),
        barriers: details.barriers,
        first_failure: details.first_failure,
        durable_events: details.durable_events,
        effect_diff: details.effect_diff,
    }
}

fn replay_command_for(case_id: &str) -> ReplayCommand {
    ReplayCommand::new(
        "cargo",
        [
            "test",
            "--package",
            "omnirepo-test-support",
            "--test",
            "failure_replay",
            "--",
            "--exact",
            case_id,
        ],
    )
}

fn spec_from_scenario(facts: &ScenarioFacts) -> FailureReplaySpec {
    let mut config = BTreeMap::new();
    config.insert("mode".to_owned(), "offline".to_owned());
    FailureReplaySpec::new(
        "test-manifest/v3",
        facts.case_id.clone(),
        facts.fixture_id.clone(),
        facts.scenario,
        facts.failure_class,
        facts.seed,
        evidence(&facts.case_id, facts.outcome),
    )
    .platform(facts.platform.clone())
    .command(CommandSummaryFixture::summary(config))
    .event_log(format!("events/{}.jsonl", facts.case_id))
    .barriers(facts.barriers.clone())
    .first_failure(facts.first_failure.clone())
    .durable_events(facts.durable_events.clone())
    .effect_diff(facts.effect_diff.clone())
    .replayable(replay_command_for(&facts.case_id))
}

fn observation_from_scenario(facts: &ScenarioFacts) -> ReplayObservation {
    ReplayObservation::new(
        facts.case_id.clone(),
        facts.fixture_id.clone(),
        facts.seed,
        facts.platform.clone(),
        facts.barriers.clone(),
        facts.first_failure.clone(),
        facts.durable_events.clone(),
    )
    .with_scenario(facts.scenario)
    .with_failure_class(facts.failure_class)
}

fn assert_independent_replay(capture: fn(u64) -> ScenarioFacts, seed: u64) {
    let saved = capture(seed);
    let bundle = spec_from_scenario(&saved)
        .build(&DiagnosticRedactor::default())
        .expect("scenario bundle");

    // Capture the hermetic control a second time. The replay observation is
    // not derived from the saved bundle.
    let replayed = capture(seed);
    let observation = observation_from_scenario(&replayed);
    assert_eq!(
        bundle
            .verify_replay(&observation)
            .expect("replay verification"),
        ReplayVerification::Reproduced,
        "scenario={:?}",
        saved.scenario
    );

    assert!(matches!(
        bundle
            .verify_replay(&observation.clone().with_seed(seed + 1))
            .expect("seed verification"),
        ReplayVerification::Diverged {
            reason: ReplayDivergence::Seed
        }
    ));

    let mut changed_barrier = replayed.clone();
    changed_barrier.barriers[0] = BarrierStep::new(
        changed_barrier.barriers[0].sequence(),
        "mutated.barrier",
        changed_barrier.barriers[0].action(),
    );
    assert!(matches!(
        bundle
            .verify_replay(&observation_from_scenario(&changed_barrier))
            .expect("barrier verification"),
        ReplayVerification::Diverged {
            reason: ReplayDivergence::BarrierSchedule
        }
    ));

    let mut changed_event = replayed.clone();
    changed_event.durable_events[0] = DurableEvent::new(
        changed_event.durable_events[0].sequence(),
        "mutated.event",
        "different",
    );
    assert!(matches!(
        bundle
            .verify_replay(&observation_from_scenario(&changed_event))
            .expect("event verification"),
        ReplayVerification::Diverged {
            reason: ReplayDivergence::DurableEventSequence
        }
    ));

    let mut changed_assertion = replayed.clone();
    changed_assertion.first_failure = AssertionFailure::new(
        changed_assertion.first_failure.event_sequence(),
        changed_assertion.first_failure.assertion_id(),
        "different-expected",
        changed_assertion.first_failure.observed(),
    );
    assert!(matches!(
        bundle
            .verify_replay(&observation_from_scenario(&changed_assertion))
            .expect("assertion verification"),
        ReplayVerification::Diverged {
            reason: ReplayDivergence::FirstFailure
        }
    ));

    assert!(matches!(
        bundle
            .verify_replay(&observation.clone().with_failure_class(
                if saved.failure_class == FailureClass::HarnessFailure {
                    FailureClass::ProductFailure
                } else {
                    FailureClass::HarnessFailure
                }
            ))
            .expect("class verification"),
        ReplayVerification::Diverged {
            reason: ReplayDivergence::FailureClass
        }
    ));
}

fn capture_process_crash(seed: u64) -> ScenarioFacts {
    let case_id = "scenario-process-crash";
    let mut fixture = LifecycleFixture::create(FixtureSpec::new(case_id, seed).retain_always())
        .expect("process crash fixture");
    let run_id = format!("crash-{seed}");
    let spec = CrashSpec::at("journal.after_flush")
        .run_id(run_id.clone())
        .with_state("stage", "journal-flushed")
        .with_state("terminal", "false");
    let mut parent = CrashableParent::spawn(&mut fixture, spec).expect("crash parent");
    parent.wait_for_boundary().expect("crash barrier");
    let crash = parent.wait().expect("crash evidence");
    let retained = RetainedState::restart(&fixture, run_id).expect("retained state");
    assert_eq!(crash.boundary, retained.boundary);
    let stage = retained.field("stage").expect("stage field").to_owned();
    let terminal = retained
        .field("terminal")
        .expect("terminal field")
        .to_owned();
    let status = crash.status.code.expect("exit status");
    let report = fixture.cleanup(FixtureOutcome::Failure);
    if report.retained {
        fs::remove_dir_all(report.root).expect("remove retained crash fixture");
    }
    scenario_facts(
        case_id,
        ScenarioDetails {
            scenario: FailureScenario::ProcessCrash,
            failure_class: FailureClass::ProductFailure,
            outcome: Outcome::Failed,
            seed,
            barriers: vec![
                BarrierStep::new(1, retained.boundary.clone(), BarrierAction::Hit),
                BarrierStep::new(2, "recovery.restart", BarrierAction::Released),
            ],
            first_failure: AssertionFailure::new(3, "process.exit", "0", status.to_string()),
            durable_events: vec![
                DurableEvent::new(1, "recovery.stage", stage),
                DurableEvent::new(2, "recovery.terminal", terminal),
            ],
            effect_diff: EffectDiff::new(vec![EffectDifference::new(
                "runs/recovery.journal",
                Some("terminal=true"),
                Some("terminal=false"),
            )]),
        },
    )
}

fn capture_concurrent_run(seed: u64) -> ScenarioFacts {
    let case_id = "scenario-concurrent-run";
    let mut fixture =
        LifecycleFixture::create(FixtureSpec::new(case_id, seed)).expect("concurrent fixture");
    let mut runs =
        ConcurrentRunControl::launch(&mut fixture, ["run-a".to_owned(), "run-b".to_owned()])
            .expect("concurrent controls");
    runs.wait_for_ready().expect("concurrent ready barrier");
    runs.release_all().expect("concurrent release barrier");
    let results = runs.join().expect("concurrent results");
    assert_eq!(results.len(), 2);
    let statuses = results
        .iter()
        .map(|result| format!("{}={:?}", result.run_id, result.status.code))
        .collect::<Vec<_>>()
        .join(",");
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());
    scenario_facts(
        case_id,
        ScenarioDetails {
            scenario: FailureScenario::ConcurrentRun,
            failure_class: FailureClass::ProductFailure,
            outcome: Outcome::Failed,
            seed,
            barriers: vec![
                BarrierStep::new(1, "concurrent.run-ready", BarrierAction::Hit),
                BarrierStep::new(2, "concurrent.release", BarrierAction::Released),
            ],
            first_failure: AssertionFailure::new(3, "lease.single-admission", "one", "two"),
            durable_events: vec![
                DurableEvent::new(1, "concurrent.run-a", statuses.clone()),
                DurableEvent::new(2, "concurrent.run-b", statuses),
            ],
            effect_diff: EffectDiff::new(vec![EffectDifference::new(
                "runs/active",
                Some("1"),
                Some("2"),
            )]),
        },
    )
}

fn capture_interrupted_journal(seed: u64) -> ScenarioFacts {
    let case_id = "scenario-interrupted-journal";
    let mut fixture =
        LifecycleFixture::create(FixtureSpec::new(case_id, seed)).expect("journal fixture");
    let journal = JournalControl::create(&mut fixture, "interrupted").expect("journal create");
    journal
        .append_record("stage=started")
        .expect("journal start");
    journal
        .append_record("stage=committing")
        .expect("journal commit");
    let complete = journal.inspect().expect("complete journal");
    journal.truncate_tail(4).expect("journal interruption");
    let truncated = journal.inspect().expect("truncated journal");
    assert_eq!(complete.tail, JournalTail::Complete);
    assert_eq!(truncated.tail, JournalTail::Truncated);
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());
    scenario_facts(
        case_id,
        ScenarioDetails {
            scenario: FailureScenario::InterruptedJournal,
            failure_class: FailureClass::HarnessFailure,
            outcome: Outcome::HarnessFailure,
            seed,
            barriers: vec![
                BarrierStep::new(1, "journal.append", BarrierAction::Hit),
                BarrierStep::new(2, "journal.tail", BarrierAction::Aborted),
            ],
            first_failure: AssertionFailure::new(3, "journal.tail", "complete", "truncated"),
            durable_events: truncated
                .records
                .iter()
                .enumerate()
                .map(|(index, record)| {
                    DurableEvent::new(index as u64 + 1, "journal.record", record)
                })
                .collect(),
            effect_diff: EffectDiff::new(vec![EffectDifference::new(
                "runs/interrupted.jsonl",
                Some("complete"),
                Some("truncated"),
            )]),
        },
    )
}

fn capture_ambiguous_git_delivery(seed: u64) -> ScenarioFacts {
    let case_id = "scenario-ambiguous-git";
    let mut fixture =
        LifecycleFixture::create(FixtureSpec::new(case_id, seed)).expect("Git fixture");
    let remote = LocalGitRemoteDouble::bind(&mut fixture, "ambiguous").expect("Git remote");
    let attempt = remote
        .begin_attempt(b"update deadbeef refs/heads/main\0")
        .expect("Git attempt");
    let accepted = remote.wait_for_accept().expect("Git accepted marker");
    remote.disconnect().expect("Git disconnect");
    let final_evidence = remote.finish().expect("Git final evidence");
    assert!(final_evidence.disconnected);
    assert!(attempt.join().expect("Git attempt join").is_empty());
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());
    scenario_facts(
        case_id,
        ScenarioDetails {
            scenario: FailureScenario::AmbiguousGitDelivery,
            failure_class: FailureClass::ProductFailure,
            outcome: Outcome::Failed,
            seed,
            barriers: vec![
                BarrierStep::new(1, "git.remote.accepted", BarrierAction::Hit),
                BarrierStep::new(2, "git.remote.disconnected", BarrierAction::Hit),
            ],
            first_failure: AssertionFailure::new(
                3,
                "git.remote.observed",
                "connected",
                "disconnected",
            ),
            durable_events: vec![
                DurableEvent::new(1, "git.payload", accepted.payload.len().to_string()),
                DurableEvent::new(2, "git.accepted", accepted.accepted.to_string()),
            ],
            effect_diff: EffectDiff::new(vec![EffectDifference::new(
                "remote/refs/heads/main",
                Some("known"),
                Some("accepted-then-disconnected"),
            )]),
        },
    )
}

fn capture_repair_attempt(seed: u64) -> ScenarioFacts {
    let case_id = "scenario-repair-attempt";
    let domain = CrossDomainFixture::new(
        DomainFixtureSpec::new(case_id, seed).expect("repair fixture identity"),
    );
    let mut fixture =
        LifecycleFixture::create(FixtureSpec::new(case_id, seed)).expect("repair fixture");
    let session = AgentDouble::start(&mut fixture, "repair", vec!["not-json".to_owned()])
        .expect("agent double");
    session.wait_for_barrier().expect("repair barrier");
    session.release().expect("repair release");
    let evidence = session.join().expect("repair evidence");
    assert!(evidence.ambient_credentials_absent);
    assert_eq!(evidence.accepted.len(), 0);
    assert_eq!(evidence.violations.len(), 1);
    let eligibility = format!("{:?}", domain.repair().eligibility());
    assert!(fixture.cleanup(FixtureOutcome::Success).leaks.is_empty());
    scenario_facts(
        case_id,
        ScenarioDetails {
            scenario: FailureScenario::RepairAttempt,
            failure_class: FailureClass::HarnessFailure,
            outcome: Outcome::HarnessFailure,
            seed,
            barriers: vec![
                BarrierStep::new(1, "agent-repair", BarrierAction::Hit),
                BarrierStep::new(2, "agent-repair", BarrierAction::Released),
            ],
            first_failure: AssertionFailure::new(3, "repair.protocol", "accepted=1", "accepted=0"),
            durable_events: vec![
                DurableEvent::new(1, "repair.eligibility", eligibility),
                DurableEvent::new(
                    2,
                    "repair.violations",
                    evidence.violations.len().to_string(),
                ),
            ],
            effect_diff: EffectDiff::new(vec![EffectDifference::new(
                "repair/attempt",
                Some("accepted"),
                Some("protocol-violation"),
            )]),
        },
    )
}

fn capture_partial_source(seed: u64) -> ScenarioFacts {
    let case_id = "scenario-partial-source";
    let context = FixtureContext::new("failure-replay", case_id, seed);
    let content = ContentFixture::new(
        &context,
        ContentModeFixture::partial(&context, "source.section").expect("partial mode"),
        b"# omnirepo:start source.section\nsource=true\n".to_vec(),
        b"local=true\n".to_vec(),
    )
    .expect("partial source fixture");
    assert_eq!(content.marker_topology(), MarkerTopologyFixture::MissingEnd);
    assert_eq!(
        content.decision(),
        ContentDecisionFixture::InvalidMarkers(MarkerTopologyFixture::MissingEnd)
    );
    scenario_facts(
        case_id,
        ScenarioDetails {
            scenario: FailureScenario::PartialSourceAvailability,
            failure_class: FailureClass::UnsupportedCapability,
            outcome: Outcome::Skipped,
            seed,
            barriers: vec![
                BarrierStep::new(1, "source.partial", BarrierAction::Armed),
                BarrierStep::new(2, "source.partial", BarrierAction::Aborted),
            ],
            first_failure: AssertionFailure::new(3, "source.markers", "paired", "missing-end"),
            durable_events: vec![DurableEvent::new(1, "source.marker", "missing-end")],
            effect_diff: EffectDiff::new(vec![EffectDifference::new(
                "source/partial",
                Some("available"),
                Some("missing-end"),
            )]),
        },
    )
}

#[test]
fn hermetic_process_crash_replay_is_independent_and_mutation_sensitive() {
    assert_independent_replay(capture_process_crash, 74_501);
}

#[test]
fn hermetic_concurrent_run_replay_is_independent_and_mutation_sensitive() {
    assert_independent_replay(capture_concurrent_run, 74_502);
}

#[test]
fn hermetic_interrupted_journal_replay_is_independent_and_mutation_sensitive() {
    assert_independent_replay(capture_interrupted_journal, 74_503);
}

#[test]
fn hermetic_ambiguous_git_delivery_replay_is_independent_and_mutation_sensitive() {
    assert_independent_replay(capture_ambiguous_git_delivery, 74_504);
}

#[test]
fn hermetic_repair_attempt_replay_is_independent_and_mutation_sensitive() {
    assert_independent_replay(capture_repair_attempt, 74_505);
}

#[test]
fn hermetic_partial_source_replay_is_independent_and_mutation_sensitive() {
    assert_independent_replay(capture_partial_source, 74_506);
}

#[test]
fn bundle_round_trip_is_byte_deterministic_and_keeps_failure_context() {
    let redactor = DiagnosticRedactor::new(["known-secret"]);
    let first = spec().build(&redactor).expect("bundle");
    let second = spec().build(&redactor).expect("bundle");

    assert_eq!(
        first.to_json().expect("json"),
        second.to_json().expect("json")
    );
    let parsed = FailureReplayBundle::from_json(&first.to_json().expect("json")).expect("parse");
    assert_eq!(parsed, first);
    assert_eq!(parsed.manifest_version(), "test-manifest/v3");
    assert_eq!(parsed.case_id(), "crash-case");
    assert_eq!(parsed.fixture_id(), "fixture-crash-7405");
    assert_eq!(parsed.seed(), 7405);
    assert_eq!(parsed.scenario(), FailureScenario::ProcessCrash);
    assert_eq!(parsed.failure_class(), FailureClass::ProductFailure);
    assert_eq!(parsed.first_failure().assertion_id(), "assertion.git.head");
    assert_eq!(parsed.durable_events().len(), 2);
    assert_eq!(parsed.peer_outcomes().len(), 1);
    assert_eq!(parsed.cleanup_failures().len(), 1);
    assert_eq!(parsed.effect_diff().entries().len(), 1);
    assert!(
        parsed
            .command_summary()
            .config()
            .get("credential")
            .unwrap()
            .contains("REDACTED")
    );
    assert!(!first.to_json().expect("json").contains("known-secret"));
    assert_eq!(
        parsed.replay_command().expect("replay").render(),
        "cargo test --package omnirepo-test-support -- --exact crash-case"
    );
}

#[test]
fn replay_verification_reproduces_seed_assertion_and_events_or_reports_typed_divergence() {
    let bundle = spec()
        .build(&DiagnosticRedactor::new(["known-secret"]))
        .expect("bundle");
    let observation = ReplayObservation::from_bundle(&bundle);
    assert_eq!(
        bundle.verify_replay(&observation).expect("verify"),
        ReplayVerification::Reproduced
    );

    let changed = observation.clone().with_seed(observation.seed() + 1);
    assert!(matches!(
        bundle.verify_replay(&changed).expect("verify"),
        ReplayVerification::Diverged { .. }
    ));
}

#[test]
fn replay_verification_rejects_a_mutated_failure_class() {
    let bundle = spec()
        .build(&DiagnosticRedactor::new(["known-secret"]))
        .expect("bundle");
    let observation =
        ReplayObservation::from_bundle(&bundle).with_failure_class(FailureClass::HarnessFailure);
    assert_eq!(
        bundle.verify_replay(&observation).expect("verify"),
        ReplayVerification::Diverged {
            reason: ReplayDivergence::FailureClass
        }
    );
}

#[test]
fn externally_supplied_evidence_diagnostic_is_sanitized_before_persistence() {
    let mut external = evidence("crash-case", Outcome::Failed);
    let terminal = external
        .events
        .iter_mut()
        .find(|event| event.terminal)
        .expect("terminal evidence event");
    terminal.diagnostic = Some(concat!("body=known-", "secret\u{1b}[31mred\u{1b}[0m").to_owned());

    let bundle = spec_with_evidence(external)
        .build(&DiagnosticRedactor::new([concat!("known-", "secret")]))
        .expect("external diagnostics must be sanitized");
    let json = bundle.to_json().expect("json");
    assert!(!json.contains("known-secret"));
    assert!(json.contains("[REDACTED]"));
    assert!(json.contains("[control-sequence]"));
}

#[test]
fn every_public_diagnostic_and_control_field_is_redacted_before_persistence() {
    let secret = concat!("known-", "secret");
    let mut external = evidence("crash-case", Outcome::Failed);
    for event in &mut external.events {
        event.identity.case_id = format!("case-{secret}");
        event.identity.suite = format!("suite-{secret}");
        event.identity.repository = format!("repository-{secret}");
        event.identity.stage = format!("stage-{secret}");
        event.identity.source_plan_config.source = format!("source-{secret}");
        event.identity.source_plan_config.plan = format!("plan-{secret}");
        event.identity.source_plan_config.config = format!("config-{secret}");
        event.identity.command = format!("command-{secret}");
        event.schema = format!("schema-{secret}");
        event.event_id = format!("event-{secret}");
        event.correlation_id = format!("correlation-{secret}");
        event.artifact = ArtifactReference {
            path: Some(format!("artifacts/{secret}")),
            replay_id: Some(format!("replay-{secret}")),
        };
        if event.terminal {
            event.diagnostic = Some(format!("body={secret}\u{1b}[31mred"));
        }
    }
    external.peer_accounting.expected_case_ids = vec![secret.to_owned()];
    external.peer_accounting.terminal_case_ids = vec![secret.to_owned()];
    external.peer_accounting.missing_case_ids = Vec::new();
    external.peer_accounting.terminal_outcomes.clear();
    external
        .projection
        .artifact_path
        .replace(format!("artifacts/{secret}"));
    external.projection.replay_id = Some(format!("replay-{secret}"));

    let mut config = BTreeMap::new();
    config.insert("credential".to_owned(), secret.to_owned());
    let bundle = spec_with_evidence(external)
        .command(omnirepo_test_support::failure_replay::CommandSummary::new(
            "cargo",
            [format!("arg-{secret}")],
            config,
        ))
        .event_log(format!("events/{secret}.jsonl"))
        .first_failure(AssertionFailure::new(
            3,
            format!("assertion-{secret}"),
            format!("expected-{secret}"),
            format!("observed-{secret}"),
        ))
        .durable_events(vec![DurableEvent::new(
            1,
            format!("event-kind-{secret}"),
            format!("event-detail-{secret}"),
        )])
        .effect_diff(EffectDiff::new(vec![EffectDifference::new(
            format!("destination/{secret}"),
            Some(format!("effect-expected-{secret}")),
            Some(format!("effect-observed-{secret}")),
        )]))
        .peers(vec![PeerOutcome::new(
            format!("peer-{secret}"),
            Outcome::Passed,
            Some(format!("peer-diagnostic-{secret}")),
        )])
        .cleanup_failures(vec![CleanupFailure::new(
            format!("cleanup-{secret}"),
            format!("cleanup-diagnostic-{secret}"),
        )])
        .replayable(ReplayCommand::new("cargo", [format!("recipe-{secret}")]));
    let json = bundle
        .build(&DiagnosticRedactor::new([secret]))
        .expect("public fields should be sanitized")
        .to_json()
        .expect("json");
    assert!(!json.contains(secret));
    assert!(json.contains("[REDACTED]"));
    assert!(json.contains("[control-sequence]"));
}

#[test]
fn chunk_boundary_secrets_and_controls_are_redacted_with_valid_accounting() {
    let bundle = spec_with_evidence(chunked_evidence())
        .build(&DiagnosticRedactor::new([concat!("known-", "secret")]))
        .expect("chunked evidence should be rebuilt");
    let json = bundle.to_json().expect("json");
    assert!(!json.contains("known-secret"));
    assert!(json.contains("[REDACTED]"));
    assert!(json.contains("[control-sequence]"));
    bundle.evidence().validate().expect("rebuilt accounting");
}

#[test]
fn non_replayable_failures_have_typed_reason_and_no_recipe() {
    let bundle = spec()
        .non_replayable(NonReplayableReason::ExternalServiceRequired)
        .build(&DiagnosticRedactor::new(["known-secret"]))
        .expect("bundle");
    assert_eq!(
        bundle.non_replayable_reason(),
        Some(NonReplayableReason::ExternalServiceRequired)
    );
    assert!(bundle.replay_command().is_none());
}

#[test]
fn bundle_rejects_failure_class_that_does_not_match_evidence() {
    let error = FailureReplaySpec::new(
        "test-manifest/v3",
        "skip-case",
        "fixture-skip-7405",
        FailureScenario::PartialSourceAvailability,
        FailureClass::UnsupportedCapability,
        7405,
        evidence("skip-case", Outcome::Skipped),
    )
    .platform(PlatformContract::new(
        "linux-ext4",
        vec![CapabilitySnapshot::unsupported("source", "partial")],
    ))
    .command(CommandSummaryFixture::summary(BTreeMap::new()))
    .event_log("events/skip-case.jsonl")
    .barriers(vec![BarrierStep::new(
        1,
        "source.ready",
        BarrierAction::Aborted,
    )])
    .first_failure(AssertionFailure::new(
        2,
        "source.available",
        "available",
        "partial",
    ))
    .durable_events(vec![DurableEvent::new(1, "source", "partial")])
    .effect_diff(EffectDiff::default())
    .peers(Vec::new())
    .cleanup_failures(Vec::new())
    .non_replayable(NonReplayableReason::UnsupportedPlatform)
    .build(&DiagnosticRedactor::default())
    .expect("unsupported bundle");
    assert_eq!(error.failure_class(), FailureClass::UnsupportedCapability);

    let mismatch = FailureReplaySpec::new(
        "test-manifest/v3",
        "skip-case",
        "fixture-skip-7405",
        FailureScenario::PartialSourceAvailability,
        FailureClass::ProductFailure,
        7405,
        evidence("skip-case", Outcome::Skipped),
    )
    .platform(PlatformContract::new(
        "linux-ext4",
        vec![CapabilitySnapshot::unsupported("source", "partial")],
    ))
    .command(CommandSummaryFixture::summary(BTreeMap::new()))
    .event_log("events/skip-case.jsonl")
    .barriers(vec![BarrierStep::new(
        1,
        "source.ready",
        BarrierAction::Aborted,
    )])
    .first_failure(AssertionFailure::new(
        2,
        "source.available",
        "available",
        "partial",
    ))
    .durable_events(vec![DurableEvent::new(1, "source", "partial")])
    .effect_diff(EffectDiff::default())
    .non_replayable(NonReplayableReason::UnsupportedPlatform)
    .build(&DiagnosticRedactor::default())
    .expect_err("failure class mismatch");
    assert!(matches!(
        mismatch,
        FailureReplayError::FailureClassMismatch { .. }
    ));
}

#[test]
fn bundle_write_failure_is_typed_and_cannot_be_treated_as_green() {
    let bundle = spec()
        .build(&DiagnosticRedactor::new(["known-secret"]))
        .expect("bundle");
    // The system temp dir on macOS lives under /var/folders, and /var is a
    // symlink there; the evidence store validates its root as symlink-free.
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    std::fs::create_dir_all(&base).expect("fixture base");
    let root = tempfile::Builder::new()
        .prefix("failure-replay-root-")
        .tempdir_in(&base)
        .expect("temp root");
    let store =
        omnirepo_test_support::test_evidence::ArtifactStore::new(root.path()).expect("store");
    bundle.write(&store, "replay.json").expect("first write");
    let error = bundle
        .write(&store, "replay.json")
        .expect_err("duplicate write");
    assert!(matches!(
        error,
        FailureReplayError::BundleWriteFailed { .. }
    ));
}

#[test]
fn persisted_bundle_rejects_unknown_fields_and_unsanitized_command_values() {
    let bundle = spec()
        .build(&DiagnosticRedactor::new(["known-secret"]))
        .expect("bundle");
    let mut value: serde_json::Value =
        serde_json::from_str(&bundle.to_json().expect("json")).expect("value");
    value["unknown"] = serde_json::Value::String("unexpected".to_owned());
    assert!(FailureReplayBundle::from_json(&serde_json::to_string(&value).expect("json")).is_err());

    let mut value: serde_json::Value =
        serde_json::from_str(&bundle.to_json().expect("json")).expect("value");
    value["command"]["config"]["token"] = serde_json::Value::String("raw-token".to_owned());
    assert!(FailureReplayBundle::from_json(&serde_json::to_string(&value).expect("json")).is_err());
}

#[test]
fn oversized_command_is_a_typed_bounded_failure() {
    let command = CommandSummaryFixture::summary(BTreeMap::new());
    let mut oversized = command.args().to_vec();
    oversized.push("x".repeat(4097));
    let error = spec()
        .command(omnirepo_test_support::failure_replay::CommandSummary::new(
            command.program(),
            oversized,
            BTreeMap::new(),
        ))
        .build(&DiagnosticRedactor::new(["known-secret"]))
        .expect_err("oversized command must fail closed");
    assert!(matches!(
        error,
        FailureReplayError::InvalidField {
            field: "command.arg",
            ..
        }
    ));
}

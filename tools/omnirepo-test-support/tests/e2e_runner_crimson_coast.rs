//! Black-box runner mechanics are tested with controlled fixture binaries.

use std::{fs, time::Duration};

use omnirepo_test_support::e2e_runner_crimson_coast::{
    E2E_RUNNER_CONTRACT_VERSION, E2eRunner, ExpectedEffects, ExpectedFile, FixtureBinarySpec,
    GitRoot, GitViolationCategory, RunnerCase, RunnerError,
};
use omnirepo_test_support::test_evidence::{DIAGNOSTIC_TRUNCATION_MARKER, EvidenceBundle, Outcome};

const TRACER_SCRIPT: &str = r##"#!/bin/sh
set -eu
test "$HOME" = "$OMNIREPO_E2E_HOME"
test "$GIT_CONFIG_NOSYSTEM" = "1"
test -z "${SSH_AUTH_SOCK+x}"
test "$OMNIREPO_E2E_OFFLINE" = "1"
printf 'tracer-ready\n'
printf 'fixture-diagnostic\n' >&2
printf 'case=%s\nreplay=%s\n' "$OMNIREPO_E2E_CASE_ID" "$OMNIREPO_E2E_REPLAY_ID" > "$OMNIREPO_E2E_EFFECTS_ROOT/result.txt"
"##;

fn tracer_case() -> RunnerCase {
    RunnerCase::new(
        "clean-root-tracer",
        FixtureBinarySpec::shell("tracer", TRACER_SCRIPT),
    )
    .expect("valid tracer case")
    .seed(4_201)
    .expected(
        ExpectedEffects::success()
            .stdout(b"tracer-ready\n".to_vec())
            .stderr(b"fixture-diagnostic\n".to_vec())
            .exact_files([ExpectedFile::path("result.txt")]),
    )
}

#[test]
fn tracer_bullet_selects_fixture_binary_and_captures_clean_environment() {
    let mut case = tracer_case();
    let expected = case.expectations().clone();
    let replay = {
        // The fixture writes its replay ID.  Build the exact expected bytes
        // after the runner has selected the stable case identity.
        let probe = RunnerCase::new(
            "clean-root-tracer",
            FixtureBinarySpec::shell("tracer", TRACER_SCRIPT),
        )
        .expect("valid probe case")
        .seed(4_201);
        let first = E2eRunner::new().run(probe).expect("probe should pass");
        let evidence = EvidenceBundle::from_jsonl(&first.evidence_json)
            .expect("canonical evidence JSONL should parse");
        let marker = first.replay_id.clone();
        assert_eq!(first.contract_version, E2E_RUNNER_CONTRACT_VERSION);
        assert_eq!(evidence.projection.outcome, Outcome::Passed);
        marker
    };
    case = case.expected(expected.exact_files([ExpectedFile::with_contents(
        "result.txt",
        format!("case=clean-root-tracer\nreplay={replay}\n").into_bytes(),
    )]));

    let report = E2eRunner::new().run(case).expect("tracer case should pass");
    assert_eq!(report.case_id, "clean-root-tracer");
    assert_eq!(report.replay_id, replay);
    assert_eq!(report.process.code, Some(0));
    assert_eq!(report.process.stdout.bytes, b"tracer-ready\n");
    assert_eq!(report.process.stderr.bytes, b"fixture-diagnostic\n");
    assert!(
        report
            .concise_status()
            .starts_with("e2e clean-root-tracer: passed")
    );
    assert!(!report.concise_status().contains("fixture-diagnostic"));
    assert!(report.artifact_root.starts_with(&report.root));
    assert!(report.effect_root.starts_with(&report.root));
    assert!(report.containment.no_outside_writes());
    assert!(report.cleanup.removed);
    assert!(!report.root.exists(), "successful roots are cleaned up");
    let evidence = EvidenceBundle::from_jsonl(&report.evidence_json)
        .expect("canonical evidence JSONL should parse");
    assert_eq!(evidence.projection.outcome, Outcome::Passed);
    assert_eq!(evidence.events.len(), 2);
}

#[test]
fn coverage_profile_output_is_routed_to_runner_profile_artifacts() {
    let case = RunnerCase::new(
        "coverage-profile-isolation",
        FixtureBinarySpec::shell(
            "coverage-profile",
            r#"#!/bin/sh
set -eu
if test -n "${LLVM_PROFILE_FILE+x}"; then
    printf 'instrumented-profile\n' > "$LLVM_PROFILE_FILE"
else
    printf 'default-profile\n' > "$PWD/default_coverage.profraw"
fi
"#,
        ),
    )
    .expect("valid coverage profile case")
    .expected(ExpectedEffects::success());

    let report = E2eRunner::new()
        .run(case)
        .expect("coverage profile must stay in runner profile artifacts");
    assert!(
        report
            .artifacts
            .iter()
            .any(|artifact| artifact.relative_path.ends_with(".profraw"))
    );
    assert!(report.artifacts.iter().any(|artifact| {
        artifact.relative_path.starts_with("artifacts/profiles/")
            && artifact.relative_path.ends_with(".profraw")
    }));
    assert!(
        !report
            .artifacts
            .iter()
            .any(|artifact| artifact.relative_path.ends_with("default_coverage.profraw"))
    );
    assert!(report.containment.no_outside_writes());
}

#[test]
fn coverage_profile_paths_are_unique_and_retained_on_failure() {
    let case = |seed| {
        RunnerCase::new(
            "coverage-profile-failure",
            FixtureBinarySpec::shell(
                "coverage-profile-failure",
                "#!/bin/sh\nprintf profile > \"$LLVM_PROFILE_FILE\"\nexit 9\n",
            ),
        )
        .expect("valid profile failure case")
        .seed(seed)
        .expected(ExpectedEffects::success())
    };
    let first = E2eRunner::new().run(case(1)).expect_err("exit must fail");
    let second = E2eRunner::new().run(case(2)).expect_err("exit must fail");
    let first = first.report().expect("first failure report");
    let second = second.report().expect("second failure report");
    let first_profile = first
        .artifacts
        .iter()
        .find(|artifact| artifact.relative_path.ends_with(".profraw"))
        .expect("first profile artifact");
    let second_profile = second
        .artifacts
        .iter()
        .find(|artifact| artifact.relative_path.ends_with(".profraw"))
        .expect("second profile artifact");
    assert_ne!(first_profile.relative_path, second_profile.relative_path);
    assert!(
        first_profile
            .relative_path
            .starts_with("artifacts/profiles/")
    );
    assert!(
        second_profile
            .relative_path
            .starts_with("artifacts/profiles/")
    );
    fs::remove_dir_all(&first.root).expect("remove first retained failure");
    fs::remove_dir_all(&second.root).expect("remove second retained failure");
}

#[test]
fn replay_identity_is_stable_for_the_same_case_and_seed() {
    let first = E2eRunner::new()
        .run(tracer_case())
        .expect("first replay should pass");
    let second = E2eRunner::new()
        .run(tracer_case())
        .expect("second replay should pass");

    assert_eq!(first.case_id, second.case_id);
    assert_eq!(first.replay_id, second.replay_id);
    assert_eq!(first.process, second.process);
    assert_eq!(first.binary.size, second.binary.size);
    assert_eq!(first.binary.fingerprint, second.binary.fingerprint);
}

#[test]
fn failed_expectations_retain_structured_evidence_and_root() {
    let case = RunnerCase::new(
        "retained-failure",
        FixtureBinarySpec::shell(
            "failure",
            "#!/bin/sh\nprintf 'unexpected\\n' > \"$OMNIREPO_E2E_EFFECTS_ROOT/extra.txt\"\nexit 7\n",
        ),
    )
    .expect("valid failure case")
    .expected(ExpectedEffects::success().exact_files([]));

    let error = E2eRunner::new()
        .run(case)
        .expect_err("unexpected process effects must fail closed");
    let RunnerError::ExpectationFailed { details, report } = error else {
        panic!("expected a structured expectation failure")
    };
    assert!(details.contains("exit status") || details.contains("effect set"));
    assert!(report.cleanup.retained);
    assert!(report.root.exists());
    let evidence = report.root.join(&report.evidence_relative_path);
    assert!(
        evidence.is_file(),
        "failed evidence must remain inspectable"
    );
    let retained = fs::read_to_string(evidence).expect("read retained evidence");
    let bundle = EvidenceBundle::from_jsonl(&retained).expect("canonical evidence JSONL");
    assert_eq!(bundle.projection.outcome, Outcome::Failed);
    fs::remove_dir_all(report.root).expect("test removes retained evidence root");
}

#[test]
fn exact_effect_assertions_reject_extra_files() {
    let case = RunnerCase::new(
        "extra-effect",
        FixtureBinarySpec::shell(
            "extra",
            "#!/bin/sh\nprintf ok > \"$OMNIREPO_E2E_EFFECTS_ROOT/allowed.txt\"\nprintf extra > \"$OMNIREPO_E2E_EFFECTS_ROOT/extra.txt\"\n",
        ),
    )
    .expect("valid extra-effect case")
    .expected(ExpectedEffects::success().exact_files([ExpectedFile::path("allowed.txt")]));

    let error = E2eRunner::new()
        .run(case)
        .expect_err("extra effects must be rejected");
    let RunnerError::ExpectationFailed { report, .. } = error else {
        panic!("expected expectation failure")
    };
    assert!(report.evidence_json.contains("extra.txt"));
    fs::remove_dir_all(report.root).expect("test removes retained evidence root");
}

#[test]
fn bounded_output_keeps_an_explicit_truncation_marker() {
    let case = RunnerCase::new(
        "bounded-output",
        FixtureBinarySpec::shell(
            "output",
            "#!/bin/sh\nprintf 1234567890123456789012345678901234567890123456789012345678901234567890\n",
        ),
    )
    .expect("valid output case")
    .expected(ExpectedEffects::success().output_limit(64));
    let report = E2eRunner::new().run(case).expect("output case should pass");
    assert!(report.process.stdout.truncated);
    assert!(
        report
            .process
            .stdout
            .bytes
            .ends_with(DIAGNOSTIC_TRUNCATION_MARKER.as_bytes())
    );
    assert!(report.process.stdout.bytes.len() + report.process.stderr.bytes.len() <= 64);
}

#[test]
fn malformed_case_inputs_fail_before_fixture_effects() {
    assert!(RunnerCase::new("../escape", FixtureBinarySpec::shell("bad", "#!/bin/sh\n"),).is_err());
    let invalid = RunnerCase::new(
        "invalid-case",
        FixtureBinarySpec::shell("bad/name", "#!/bin/sh\n"),
    )
    .expect("case ID is valid")
    .expected(ExpectedEffects::default().effect_root("../outside"));
    assert!(E2eRunner::new().run(invalid).is_err());
}

#[test]
fn output_limit_rejects_values_above_canonical_bound() {
    let case = RunnerCase::new(
        "oversized-limit",
        FixtureBinarySpec::shell("limit", "#!/bin/sh\nexit 0\n"),
    )
    .expect("valid case")
    .expected(ExpectedEffects::success().output_limit(usize::MAX));

    let error = E2eRunner::new()
        .run(case)
        .expect_err("the runner must reject an unbounded output limit");
    assert!(error.to_string().contains("output limit"));
}

#[test]
#[cfg(unix)]
fn timeout_terminates_the_process_group_and_retains_replayable_evidence() {
    let case = RunnerCase::new(
        "timeout-tree",
        FixtureBinarySpec::shell(
            "hang",
            "#!/bin/sh\n( sleep 30; printf late > \"$OMNIREPO_E2E_OUTSIDE_CANARY/late.txt\" ) &\nwhile :; do sleep 1; done\n",
        ),
    )
    .expect("valid case")
    .expected(ExpectedEffects::success().timeout(Duration::from_millis(100)));

    let started = std::time::Instant::now();
    let error = E2eRunner::new()
        .run(case)
        .expect_err("a hanging process must time out");
    assert!(started.elapsed() < Duration::from_secs(5));
    let report = error.report().expect("timeout must retain a report");
    assert!(report.process.timed_out);
    assert!(report.process.tree_terminated);
    assert!(report.process.reaped);
    assert!(
        !report
            .containment
            .outside_after
            .entries
            .iter()
            .any(|entry| entry.relative_path == "late.txt")
    );
    assert!(
        report.root.exists(),
        "timeout evidence root must be retained"
    );
    fs::remove_dir_all(&report.root).expect("remove retained timeout evidence");
}

#[test]
#[cfg(unix)]
fn controlled_outside_canary_and_unauthorized_home_write_fail_closed() {
    let case = RunnerCase::new(
        "outside-home-write",
        FixtureBinarySpec::shell(
            "outside",
            "#!/bin/sh\nprintf outside > \"$OMNIREPO_E2E_OUTSIDE_CANARY/out.txt\"\nprintf home > \"$HOME/unauthorized.txt\"\n",
        ),
    )
    .expect("valid case")
    .expected(ExpectedEffects::success());

    let error = E2eRunner::new()
        .run(case)
        .expect_err("outside and HOME writes must not be silently accepted");
    let report = error
        .report()
        .expect("containment failure must retain evidence");
    assert!(!report.containment.outside_paths.is_empty());
    assert!(
        report
            .containment
            .unauthorized_paths
            .iter()
            .any(|path| { path.to_string_lossy().contains("unauthorized.txt") })
    );
    fs::remove_dir_all(&report.root).expect("remove retained containment evidence");
}

#[test]
#[cfg(unix)]
fn unauthorized_config_source_destination_and_remote_writes_are_observed() {
    let case = RunnerCase::new(
        "unauthorized-roots",
        FixtureBinarySpec::shell(
            "unauthorized-roots",
            "#!/bin/sh\nprintf bad > \"$OMNIREPO_E2E_CONFIG.unauthorized\"\nprintf bad > \"$OMNIREPO_E2E_SOURCE/unauthorized.txt\"\nprintf bad > \"$OMNIREPO_E2E_DESTINATION/unauthorized.txt\"\nprintf bad > \"$OMNIREPO_E2E_REMOTE/unauthorized.txt\"\n",
        ),
    )
    .expect("valid root case")
    .expected(ExpectedEffects::success());
    let error = E2eRunner::new()
        .run(case)
        .expect_err("writes across authority roots must fail closed");
    let report = error.report().expect("root-write evidence");
    for name in ["unauthorized.txt", "config.yaml.unauthorized"] {
        assert!(
            report
                .containment
                .unauthorized_paths
                .iter()
                .any(|path| path.to_string_lossy().contains(name))
        );
    }
    fs::remove_dir_all(&report.root).expect("remove retained root-write evidence");
}

#[test]
#[cfg(unix)]
fn symlink_and_nonregular_effects_are_rejected_with_retained_evidence() {
    let case = RunnerCase::new(
        "nonregular-effect",
        FixtureBinarySpec::shell(
            "nonregular",
            "#!/bin/sh\nln -s \"$OMNIREPO_E2E_OUTSIDE_CANARY\" \"$OMNIREPO_E2E_EFFECTS_ROOT/escape\"\n",
        ),
    )
    .expect("valid case")
    .expected(ExpectedEffects::success());

    let error = E2eRunner::new()
        .run(case)
        .expect_err("symlink effects must fail closed");
    let report = error
        .report()
        .expect("symlink failure must retain evidence");
    assert!(
        report
            .containment
            .nonregular_paths
            .iter()
            .any(|path| { path.to_string_lossy().contains("escape") })
    );
    fs::remove_dir_all(&report.root).expect("remove retained nonregular evidence");
}

#[test]
#[cfg(unix)]
fn traversal_out_of_effect_root_is_rejected_with_retained_evidence() {
    let case = RunnerCase::new(
        "traversal-effect",
        FixtureBinarySpec::shell(
            "traversal",
            "#!/bin/sh\nprintf bad > \"$OMNIREPO_E2E_EFFECTS_ROOT/../traversal.txt\"\n",
        ),
    )
    .expect("valid traversal case")
    .expected(ExpectedEffects::success());

    let error = E2eRunner::new()
        .run(case)
        .expect_err("effect traversal must fail closed");
    let report = error.report().expect("traversal evidence must be retained");
    assert!(
        report
            .containment
            .unauthorized_paths
            .iter()
            .any(|path| path.to_string_lossy().contains("traversal.txt"))
    );
    fs::remove_dir_all(&report.root).expect("remove retained traversal evidence");
}

#[test]
#[cfg(unix)]
fn signal_and_spawn_failures_are_typed_and_replayable() {
    let signal_case = RunnerCase::new(
        "signal-child",
        FixtureBinarySpec::shell("signal", "#!/bin/sh\nkill -TERM $$\n"),
    )
    .expect("valid signal case")
    .expected(ExpectedEffects::success());
    let signal_error = E2eRunner::new()
        .run(signal_case)
        .expect_err("signal termination must fail the case");
    let signal_report = signal_error.report().expect("signal report is retained");
    assert_eq!(signal_report.process.signal, Some(15));
    fs::remove_dir_all(&signal_report.root).expect("remove retained signal evidence");

    let invalid_executable = tempfile::NamedTempFile::new().expect("invalid executable fixture");
    fs::write(invalid_executable.path(), [0xff, 0x00, 0x01]).expect("write invalid executable");
    let spawn_case = RunnerCase::new(
        "spawn-failure",
        FixtureBinarySpec::existing("invalid", invalid_executable.path()),
    )
    .expect("case identity is valid");
    let spawn_error = E2eRunner::new()
        .run(spawn_case)
        .expect_err("missing executable must be a typed spawn failure");
    assert!(spawn_error.is_spawn_failure());
    let report = spawn_error
        .report()
        .expect("spawn evidence must be retained");
    assert!(report.root.exists());
    fs::remove_dir_all(&report.root).expect("remove retained spawn evidence");
}

#[test]
#[cfg(unix)]
fn binary_control_and_secret_output_is_canonicalized_before_artifacts() {
    let case = RunnerCase::new(
        "hostile-output",
        FixtureBinarySpec::shell(
            "hostile-output",
            "#!/bin/sh\nprintf 'token=secret-value\\033[31m\\377\\n'\n",
        ),
    )
    .expect("valid hostile output case")
    .expected(
        ExpectedEffects::success()
            .redact_secret("secret-value")
            .stdout(b"not-hostile\n".to_vec())
            .output_limit(128),
    );
    let error = E2eRunner::new()
        .run(case)
        .expect_err("hostile output assertion should retain sanitized evidence");
    let report = error.report().expect("output evidence report");
    let stdout = fs::read(report.artifact_root.join("hostile-output/stdout.bin"))
        .expect("sanitized stdout artifact");
    assert!(
        !stdout
            .windows(b"secret-value".len())
            .any(|window| window == b"secret-value")
    );
    assert!(!stdout.contains(&0x1b));
    assert!(stdout.len() <= 128);
    assert!(
        stdout.len()
            + fs::read(report.artifact_root.join("hostile-output/stderr.bin"))
                .expect("sanitized stderr artifact")
                .len()
            <= 128
    );
    fs::remove_dir_all(&report.root).expect("remove retained hostile output evidence");
}

#[test]
#[cfg(unix)]
fn git_refs_are_snapshotted_and_unexpected_changes_fail_closed() {
    let case = RunnerCase::new(
        "git-ref-drift",
        FixtureBinarySpec::shell(
            "git-drift",
            "#!/bin/sh\ngit -C \"$OMNIREPO_E2E_DESTINATION\" config user.name fixture\ngit -C \"$OMNIREPO_E2E_DESTINATION\" config user.email fixture@invalid\nprintf drift >> \"$OMNIREPO_E2E_DESTINATION/tracked.txt\"\ngit -C \"$OMNIREPO_E2E_DESTINATION\" add tracked.txt\ngit -C \"$OMNIREPO_E2E_DESTINATION\" commit --quiet -m drift\n",
        ),
    )
    .expect("valid git case")
    .expected(ExpectedEffects::success());
    let error = E2eRunner::new()
        .run(case)
        .expect_err("unexpected destination ref changes must fail");
    let report = error.report().expect("Git drift must retain evidence");
    assert_ne!(report.git.destination_before, report.git.destination_after);
    assert!(report.git.unexpected_changes);
    fs::remove_dir_all(&report.root).expect("remove retained Git evidence");
}

#[test]
#[cfg(unix)]
fn expected_git_ref_changes_are_exact_and_allowed() {
    let case = RunnerCase::new(
        "expected-git-ref",
        FixtureBinarySpec::shell(
            "expected-git-ref",
            "#!/bin/sh\ngit -C \"$OMNIREPO_E2E_DESTINATION\" update-ref -d refs/heads/master\n",
        ),
    )
    .expect("valid expected Git case")
    .expected(
        ExpectedEffects::success()
            .expect_git_ref(GitRoot::Destination, "refs/heads/master", None)
            .expect_git_ref(GitRoot::Destination, "HEAD", None),
    );

    let report = E2eRunner::new()
        .run(case)
        .expect("declared Git ref changes should pass");
    assert!(!report.git.unexpected_changes);
    assert_eq!(
        report.git.destination_after.refs.get("refs/heads/master"),
        None
    );
    assert!(report.containment.no_outside_writes());
    assert!(report.cleanup.removed);
}

#[test]
#[cfg(unix)]
fn effect_paths_with_spaces_are_contained() {
    let case = RunnerCase::new(
        "space-path",
        FixtureBinarySpec::shell(
            "space-path",
            "#!/bin/sh\nprintf 'non-utf8=\\377\\n' > \"$OMNIREPO_E2E_EFFECTS_ROOT/space file.txt\"\n",
        ),
    )
    .expect("valid path case")
    .expected(
        ExpectedEffects::success()
            .effect_root("effects/path with spaces")
            .exact_files([ExpectedFile::path("space file.txt")]),
    );
    let report = E2eRunner::new()
        .run(case)
        .expect("spaces and non-UTF8 effect bytes are supported");
    assert!(report.containment.no_outside_writes());
    assert!(report.process.success());
}

#[test]
#[cfg(unix)]
fn ref_only_expectation_rejects_git_admin_drift_by_typed_path() {
    let case = RunnerCase::new(
        "git-admin-drift",
        FixtureBinarySpec::shell(
            "git-admin-drift",
            "#!/bin/sh\nset -eu\ngit -C \"$OMNIREPO_E2E_DESTINATION\" update-ref refs/tags/unrelated refs/heads/master\ngit -C \"$OMNIREPO_E2E_DESTINATION\" update-ref -d refs/heads/master\nprintf changed > \"$OMNIREPO_E2E_DESTINATION/.git/config\"\nprintf changed > \"$OMNIREPO_E2E_DESTINATION/.git/hooks/pre-commit.sample\"\nprintf changed > \"$OMNIREPO_E2E_DESTINATION/.git/index\"\nprintf '# pack-refs with: peeled\\n' > \"$OMNIREPO_E2E_DESTINATION/.git/packed-refs\"\n",
        ),
    )
    .expect("valid Git administrative drift case")
    .expected(
        ExpectedEffects::success()
            .expect_git_ref(GitRoot::Destination, "refs/heads/master", None)
            .expect_git_ref(GitRoot::Destination, "HEAD", None),
    );

    let error = E2eRunner::new()
        .run(case)
        .expect_err("RefOnly must not authorize Git administrative files");
    let RunnerError::ExpectationFailed { report, .. } = error else {
        panic!("expected typed Git administrative expectation failure");
    };
    assert!(
        report
            .git
            .administrative_violations
            .iter()
            .any(|violation| {
                violation.root == GitRoot::Destination
                    && violation.category == GitViolationCategory::Config
                    && violation.path.display() == "config"
            })
    );
    assert!(
        report
            .git
            .administrative_violations
            .iter()
            .any(|violation| {
                violation.category == GitViolationCategory::UnrelatedRef
                    && violation.path.display() == "refs/tags/unrelated"
            })
    );
    assert!(
        report
            .git
            .administrative_violations
            .iter()
            .any(|violation| {
                violation.category == GitViolationCategory::PackedRefs
                    && violation.path.display() == "packed-refs"
            })
    );
    assert!(
        report
            .git
            .administrative_violations
            .iter()
            .any(|violation| {
                violation.category == GitViolationCategory::Hook
                    && violation.path.display() == "hooks/pre-commit.sample"
            })
    );
    assert!(
        report
            .git
            .administrative_violations
            .iter()
            .any(|violation| {
                violation.category == GitViolationCategory::Index
                    && violation.path.display() == "index"
            })
    );
    fs::remove_dir_all(&report.root).expect("remove retained Git administrative evidence");
}

#[test]
#[cfg(unix)]
fn unreachable_git_object_is_not_authorized_by_a_ref_expectation() {
    let case = RunnerCase::new(
        "git-unreachable-object",
        FixtureBinarySpec::shell(
            "git-unreachable-object",
            "#!/bin/sh\nset -eu\nprintf unreachable | git -C \"$OMNIREPO_E2E_DESTINATION\" hash-object -w --stdin >/dev/null\n",
        ),
    )
    .expect("valid unreachable object case")
    .expected(ExpectedEffects::success());

    let error = E2eRunner::new()
        .run(case)
        .expect_err("unreachable Git objects must fail closed");
    let RunnerError::ExpectationFailed { report, .. } = error else {
        panic!("expected unreachable object expectation failure");
    };
    assert!(
        report
            .git
            .administrative_violations
            .iter()
            .any(|violation| {
                violation.root == GitRoot::Destination
                    && violation.category == GitViolationCategory::Object
                    && violation.path.display().starts_with("objects/")
            })
    );
    fs::remove_dir_all(&report.root).expect("remove retained unreachable object evidence");
}

#[test]
#[cfg(unix)]
fn new_hard_link_to_the_controlled_canary_fails_without_byte_drift() {
    let case = RunnerCase::new(
        "hard-link-canary",
        FixtureBinarySpec::shell(
            "hard-link-canary",
            "#!/bin/sh\nset -eu\nln \"$OMNIREPO_E2E_OUTSIDE_CANARY/sentinel\" \"$OMNIREPO_E2E_EFFECTS_ROOT/alias\"\n",
        ),
    )
    .expect("valid hard-link case")
    .expected(ExpectedEffects::success());

    let error = E2eRunner::new()
        .run(case)
        .expect_err("a new hard link crossing the canary boundary must fail");
    let report = error.report().expect("hard-link evidence must be retained");
    assert!(!report.containment.hard_link_paths.is_empty());
    assert!(report.containment.hard_link_paths.iter().any(|path| {
        path.to_string_lossy().contains("alias") || path.to_string_lossy().contains("sentinel")
    }));
    fs::remove_dir_all(&report.root).expect("remove retained hard-link evidence");
}

#[test]
#[cfg(unix)]
fn raw_non_utf8_expected_file_preserves_lossless_identity() {
    let raw_name = b"raw-\xff.txt".to_vec();
    let expected = ExpectedFile::raw_path_with_contents(raw_name.clone(), b"raw-bytes\n".to_vec())
        .expect("raw Unix path identity is valid");
    let case = RunnerCase::new(
        "raw-path-identity",
        FixtureBinarySpec::shell(
            "raw-path-identity",
            "#!/bin/sh\nset -eu\nname=$(printf 'raw-\\377.txt')\nprintf 'raw-bytes\\n' > \"$OMNIREPO_E2E_EFFECTS_ROOT/$name\"\n",
        ),
    )
    .expect("valid raw path case")
    .expected(ExpectedEffects::success().exact_files([expected]));

    let report = E2eRunner::new()
        .run(case)
        .expect("raw non-UTF-8 filename should match exactly");
    assert!(
        report
            .artifacts
            .iter()
            .any(|artifact| artifact.relative_path.contains("raw-\\xff.txt"))
    );
    assert!(report.containment.no_outside_writes());
}

#[test]
#[cfg(target_os = "linux")]
fn strict_supervisor_reaps_setsid_escape_before_snapshot() {
    let case = RunnerCase::new(
        "setsid-escape",
        FixtureBinarySpec::shell(
            "setsid-escape",
            "#!/bin/sh\nset -eu\nsetsid sh -c 'printf started > \"$OMNIREPO_E2E_EFFECTS_ROOT/escaped-started\"; sleep 30; printf late > \"$OMNIREPO_E2E_OUTSIDE_CANARY/escaped-late\"' &\nwhile [ ! -f \"$OMNIREPO_E2E_EFFECTS_ROOT/escaped-started\" ]; do sleep 0.01; done\nwhile :; do sleep 1; done\n",
        ),
    )
    .expect("valid setsid escape case")
    .expected(ExpectedEffects::success().timeout(Duration::from_millis(250)));

    let error = E2eRunner::new()
        .run(case)
        .expect_err("setsid escape should time out and retain evidence");
    let report = error.report().expect("setsid timeout report");
    assert!(report.process.tree_terminated);
    assert!(report.process.reaped);
    assert!(report.process.descendants_detected);
    assert!(
        !report
            .containment
            .outside_after
            .entries
            .iter()
            .any(|entry| entry.relative_path == "escaped-late")
    );
    fs::remove_dir_all(&report.root).expect("remove retained setsid evidence");
}

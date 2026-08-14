//! End-to-end contracts for the publishable product and private Beads tooling.
//!
//! These tests intentionally cross the Cargo/package/process boundaries.  They
//! are the final proof that the private developer workflow is usable without
//! widening the shipped product surface or mutating the live tracker.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn temporary_repo(case_id: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("omnirepo-integration-{case_id}-{stamp}"));
    fs::create_dir_all(root.join(".beads")).expect("create isolated Beads directory");
    root
}

fn package_listing() -> Vec<String> {
    let output = Command::new("cargo")
        .args(["package", "--list", "--allow-dirty", "--locked"])
        .current_dir(repository_root())
        .output()
        .expect("run locked package listing");
    assert!(
        output.status.success(),
        "case=package-list command=cargo package --list --allow-dirty --locked\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("package listing is UTF-8")
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn product_package_contains_runtime_only_files() {
    let listing = package_listing();
    let allowed = [
        "CHANGELOG.md",
        "CONSTITUTION.md",
        "Cargo.lock",
        "Cargo.toml",
        "LICENSE",
        "README.md",
        "src/main.rs",
        "src/configuration/mod.rs",
        "src/configuration/discovery.rs",
        "src/configuration/yaml_subset.rs",
        "src/lifecycle/mod.rs",
        "src/lifecycle/event.rs",
        "src/lifecycle/exit_status.rs",
        "src/lifecycle/fleet_permits.rs",
        "src/lifecycle/fleet_scenarios.rs",
        "src/lifecycle/fleet_profile.rs",
        "src/lifecycle/fleet_generators.rs",
        "src/lifecycle/initial_pass.rs",
        "src/lifecycle/initial_sync.rs",
        "src/lifecycle/git_delivery.rs",
        "src/lifecycle/single_repo_pass.rs",
        "src/lifecycle/work_mapping.rs",
        "src/lifecycle/fleet_fanout.rs",
        "src/lifecycle/fleet_collector.rs",
        "src/lifecycle/fleet_app.rs",
        "src/lifecycle/fleet_catalog.rs",
        "src/lifecycle/fleet_declarations.rs",
        "src/lifecycle/fleet_binding.rs",
        "src/lifecycle/fleet_policy.rs",
        "src/lifecycle/fleet_planning.rs",
        "src/lifecycle/fleet_composition.rs",
        "src/lifecycle/fleet_snapshot.rs",
        "src/lifecycle/fleet_runner.rs",
        "src/lifecycle/fleet_repair.rs",
        "src/lifecycle/repair_causation.rs",
        "src/lifecycle/repair_reserve.rs",
        "src/lifecycle/repair_snapshot.rs",
        "src/lifecycle/repair_delta.rs",
        "src/lifecycle/repair_reapply.rs",
        "src/lifecycle/repair_deliver.rs",
        "src/lifecycle/repair_selection.rs",
        "src/lifecycle/repair_fallback.rs",
        "src/lifecycle/repair_fold.rs",
        "src/lifecycle/migration_decision.rs",
        "src/lifecycle/lifecycle_model.rs",
        "src/lifecycle/hostile_fixtures.rs",
        "src/lifecycle/hostile_process_fixtures.rs",
        "src/lifecycle/model_generation.rs",
        "src/lifecycle/model_property_suite.rs",
        "src/lifecycle/repair_execute.rs",
        "src/lifecycle/repair_fixture_tests.rs",
        "src/lifecycle/repair_classify.rs",
        "src/lifecycle/fleet_fixture_tests.rs",
        "src/lifecycle/fleet_regression_tests.rs",
        "src/lifecycle/hostile_acceptance_tests.rs",
        "src/lifecycle/evidence_hostile_tests.rs",
        "src/lifecycle/platform_acceptance_tests.rs",
        "src/lifecycle/process_acceptance_tests.rs",
        "src/lifecycle/verify_and_gate.rs",
        "src/lifecycle/initial_pass/transition.rs",
        "src/lifecycle/invocation.rs",
        "src/lifecycle/journal.rs",
        "src/lifecycle/replay.rs",
        "src/lifecycle/replace.rs",
        "src/lifecycle/admission.rs",
        "src/lifecycle/acceptance_journeys.rs",
        "src/lifecycle/adapters.rs",
        "src/lifecycle/agent_confinement.rs",
        "src/lifecycle/agent_runtime.rs",
        "src/lifecycle/cancellation.rs",
        "src/lifecycle/check_runner.rs",
        "src/lifecycle/verification_gate.rs",
        "src/lifecycle/verification_fixture_tests.rs",
        "src/lifecycle/verifier_confinement.rs",
        "src/lifecycle/command_spec.rs",
        "src/lifecycle/agent_framing.rs",
        "src/lifecycle/commit_journal.rs",
        "src/lifecycle/source_catalog.rs",
        "src/lifecycle/nested_permits.rs",
        "src/lifecycle/remote_push.rs",
        "src/lifecycle/source_extraction.rs",
        "src/lifecycle/scheduler.rs",
        "src/lifecycle/run_summary.rs",
        "src/lifecycle/record_finalize.rs",
        "src/lifecycle/plan_selection.rs",
        "src/lifecycle/terminal_projection.rs",
        "src/lifecycle/plan_builder.rs",
        "src/lifecycle/output_guard.rs",
        "src/lifecycle/platform_matrix.rs",
        "src/managed_content/delimiters.rs",
        "src/lifecycle/replacement_requests.rs",
        "src/lifecycle/diagnostics.rs",
        "src/managed_content/partial_scan.rs",
        "src/managed_content/whole_file.rs",
        "src/lifecycle/preflight.rs",
        "src/managed_content/section_builder.rs",
        "src/managed_content/representation.rs",
        "src/lifecycle/repository_preflight.rs",
        "src/managed_content/section_append.rs",
        "src/managed_content/section_fixture_tests.rs",
        "src/lifecycle/diagnostic_aggregation.rs",
        "src/lifecycle/sync_plan.rs",
        "src/lifecycle/push_reconcile.rs",
        "src/source/catalog_state.rs",
        "src/source/item_resolution.rs",
        "src/source/extraction.rs",
        "src/lifecycle/stages.rs",
        "src/lifecycle/transaction_evidence.rs",
        "src/lifecycle/remote_target.rs",
        "src/lifecycle/run_record.rs",
        "src/managed_content/mod.rs",
        "src/managed_content/compare.rs",
        "src/managed_content/transaction/transaction.rs",
        "src/managed_content/transaction/state.rs",
        "src/managed_content/transaction/recovery.rs",
        "src/managed_content/transaction/plan.rs",
        "src/managed_content/transaction/mod.rs",
        "src/managed_content/transaction/errors.rs",
        "src/managed_content/transaction/candidates.rs",
        "src/managed_content/transaction/artifact.rs",
        "src/platform/mod.rs",
        "src/platform/authority/mod.rs",
        "src/platform/authority/roots.rs",
        "src/platform/authority/paths.rs",
        "src/platform/authority/identity.rs",
        "src/platform/authority/backend.rs",
        "src/repository/mod.rs",
        "src/repository/capture.rs",
        "src/repository/policy.rs",
        "src/repository/git_index.rs",
        "src/repository/manifest.rs",
        "src/repository/operation_commit.rs",
        "src/repository/operation_tree.rs",
        "src/repository/policy_loader.rs",
        "src/repository/revalidate.rs",
        "src/repository/state/targets.rs",
        "src/repository/state/mod.rs",
        "src/repository/state/identities.rs",
        "src/repository/state/git_facts.rs",
        "src/repository/state/facts.rs",
        "src/repository/state/domain_error.rs",
        "src/repository/state/delta.rs",
        "src/repository/state/causation.rs",
        "src/repository/state/canonical.rs",
        "src/source/mod.rs",
        "src/source/acquisition.rs",
        "src/source/declarations.rs",
        "src/source/publish.rs",
        "src/source/snapshot.rs",
        // Cargo injects these two metadata files while packaging. They are
        // generated by Cargo and cannot be excluded from `cargo package`.
        ".cargo_vcs_info.json",
        "Cargo.toml.orig",
    ];
    let forbidden = [
        ".beads/",
        ".cargo/",
        ".github/",
        ".codex/",
        ".claude/",
        "canon/",
        "docs/",
        "scripts/",
        "tests/",
        "tools/",
        ".gitignore",
        ".pre-commit-config.yaml",
        "AGENTS.md",
        "CLAUDE.md",
        "cog.toml",
    ];
    let unexpected = listing
        .iter()
        .filter(|entry| !allowed.contains(&entry.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let package_message = format!(
        "case=package-runtime-allowlist command=cargo package --list --allow-dirty --locked\nunexpected entries={unexpected:?}\nreplay=rtk cargo package --list --allow-dirty --locked"
    );
    assert!(unexpected.is_empty(), "{package_message}");
    for path in [
        "src/configuration/unit_tests.rs",
        "src/source/snapshot_tests.rs",
        "src/repository/policy_tests.rs",
    ] {
        assert!(
            !listing.iter().any(|entry| entry == path),
            "case=package-test-only-exclusion path={path} command=cargo package --list --allow-dirty --locked\nartifact=package-list\nreplay=rtk cargo package --list --allow-dirty --locked"
        );
    }
    for path in forbidden {
        let exclusion_message = format!(
            "case=package-exclusion path={path} command=cargo package --list --allow-dirty --locked\nartifact=package-list\nreplay=rtk cargo package --list --allow-dirty --locked"
        );
        assert!(
            !listing
                .iter()
                .any(|entry| entry == path || entry.starts_with(path)),
            "{exclusion_message}"
        );
    }
}

#[test]
fn validator_is_read_only_and_independent_of_br() {
    let root = temporary_repo("validator-read-only");
    let tracked = root.join(".beads/issues.jsonl");
    let original = b"{\"id\":\"ordinary\",\"status\":\"open\",\"labels\":[]}\n";
    fs::write(&tracked, original).expect("write isolated tracker export");
    let manifest = repository_root().join("tools/omnirepo-dev/Cargo.toml");
    let output = Command::new("cargo")
        .args(["run", "--quiet", "--locked", "--manifest-path"])
        .arg(&manifest)
        .args(["--", "validate-decisions", "--json"])
        .current_dir(&root)
        .env("BEADS_JSONL", &tracked)
        .output()
        .expect("run Rust validator without br");
    let validator_message = format!(
        "case=validator-read-only seed=3101 command=omnirepo-dev validate-decisions --json\nstdout={}\nstderr={}\nartifact={}\nreplay=rtk cargo test --locked --test workspace_tooling_integration validator_is_read_only_and_independent_of_br -- --nocapture",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        tracked.display()
    );
    assert!(output.status.success(), "{validator_message}");
    assert!(
        output.stderr.is_empty(),
        "JSON diagnostics must stay on stdout"
    );
    assert_eq!(fs::read(&tracked).expect("read tracker export"), original);
    fs::remove_dir_all(root).expect("remove isolated validator fixture");
}

#[test]
fn developer_dispatcher_has_stable_failure_and_replay_projection() {
    let manifest = repository_root().join("tools/omnirepo-dev/Cargo.toml");
    let output = Command::new("cargo")
        .args(["run", "--quiet", "--locked", "--manifest-path"])
        .arg(manifest)
        .args(["--", "unsupported-integration-command"])
        .output()
        .expect("run developer dispatcher");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).expect("diagnostic is UTF-8");
    assert!(stderr.contains("unsupported developer command"));
    assert!(stderr.ends_with('\n'));
}

#[allow(dead_code)]
fn _assert_fixture_is_inside_temp_root(path: &Path) {
    assert!(path.starts_with(std::env::temp_dir()));
}

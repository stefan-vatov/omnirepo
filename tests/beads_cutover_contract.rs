//! Wiring contract for the Rust-only Beads workflow.
//!
//! This contract keeps repository-owned callers on the Rust developer tool and
//! rejects mutable external workflow authority. Shell scripts may remain as
//! historical data only when they are not active callers.

use std::fs;
use std::path::{Path, PathBuf};

use yaml_serde::Value;

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(path: &str) -> String {
    fs::read_to_string(repository_root().join(path))
        .unwrap_or_else(|error| panic!("read repository file {path}: {error}"))
}

fn mapping_value<'a>(mapping: &'a yaml_serde::Mapping, key: &str) -> Option<&'a Value> {
    mapping
        .iter()
        .find_map(|(candidate, value)| (candidate.as_str() == Some(key)).then_some(value))
}

fn workflow_violations(content: &str) -> Vec<&'static str> {
    let mut violations = Vec::new();
    if content.contains("stefan-vatov/gh-workflows")
        || content.contains("/.github/workflows/")
        || content.contains("@main")
        || content.contains("@master")
    {
        violations.push("mutable or external reusable workflow authority");
    }
    if !content.contains("permissions:\n  contents: read") {
        violations.push("workflow permissions are not bounded to contents: read");
    }
    if !content.contains("omnirepo-dev")
        || !content.contains("--manifest scripts/quality-manifest.json")
        || !content.contains("--profile stable --json")
    {
        violations.push("repository-owned aggregate quality runner is missing");
    }
    violations
}

#[test]
fn active_workflow_wiring_invokes_private_rust_tool_only() {
    let sources = [
        ".pre-commit-config.yaml",
        ".github/workflows/pre-commit-hooks.yml",
        "scripts/quality-manifest.json",
        "docs/quality-manifest.md",
    ];

    for path in sources {
        let content = read(path);
        assert!(
            content.contains("omnirepo-dev"),
            "{path} must invoke the private Rust developer tool"
        );
        for stem in [
            "agent-plan",
            "validate-decisions",
            "transition-matrix",
            "beads_decision_validator",
            "decision-plan",
            "beads_compatibility_fixtures",
            "beads_compatibility_mutation",
        ] {
            let obsolete = format!("{stem}.sh");
            assert!(
                !content.contains(&obsolete),
                "{path} must not invoke obsolete Beads shell glue: {obsolete}"
            );
        }
    }
}

#[test]
fn pre_commit_hook_mappings_are_not_exact_duplicates_within_a_repository() {
    let source = read(".pre-commit-config.yaml");
    let document =
        yaml_serde::from_str::<Value>(&source).expect(".pre-commit-config.yaml must be valid YAML");
    let root = document
        .as_mapping()
        .expect(".pre-commit-config.yaml root must be a mapping");
    let repositories = mapping_value(root, "repos")
        .and_then(Value::as_sequence)
        .expect(".pre-commit-config.yaml must define a repos sequence");

    for (repository_index, repository) in repositories.iter().enumerate() {
        let repository_map = repository
            .as_mapping()
            .unwrap_or_else(|| panic!("repository {repository_index} must be a mapping"));
        let repository_name = mapping_value(repository_map, "repo")
            .and_then(Value::as_str)
            .unwrap_or("<local>");
        let hooks = mapping_value(repository_map, "hooks")
            .and_then(Value::as_sequence)
            .unwrap_or_else(|| panic!("repository {repository_name} must define hooks"));
        let mut seen_hooks = Vec::new();

        for (hook_index, hook) in hooks.iter().enumerate() {
            let hook_map = hook.as_mapping().unwrap_or_else(|| {
                panic!("repository {repository_name} hook {hook_index} must be a mapping")
            });
            let hook_id = mapping_value(hook_map, "id")
                .and_then(Value::as_str)
                .unwrap_or_else(|| {
                    panic!("repository {repository_name} hook {hook_index} must define an id")
                });
            assert!(
                !seen_hooks.iter().any(|previous| previous == hook),
                "repository {repository_name} contains an exact duplicate hook mapping for {hook_id}"
            );
            seen_hooks.push(hook.clone());
        }
    }
}

#[test]
fn obsolete_shell_entry_points_are_not_executable_files() {
    for stem in ["agent-plan", "validate-decisions", "transition-matrix"] {
        let path = repository_root().join(".beads").join(format!("{stem}.sh"));
        assert!(
            !path.exists(),
            "obsolete Beads shell entry point remains: {}",
            path.display()
        );
    }

    for stem in [
        "beads_decision_validator",
        "decision-plan",
        "beads_compatibility_fixtures",
        "beads_compatibility_mutation",
    ] {
        let path = repository_root().join("tests").join(format!("{stem}.sh"));
        assert!(
            !path.exists(),
            "obsolete Beads shell test remains: {}",
            path.display()
        );
    }
}

#[test]
fn pre_commit_workflow_is_repository_owned_and_read_only() {
    let workflow = read(".github/workflows/pre-commit-hooks.yml");
    let violations = workflow_violations(&workflow);
    assert!(
        violations.is_empty(),
        "pre-commit workflow contract violations: {violations:?}"
    );
}

#[test]
fn mutable_reusable_workflow_mutation_is_rejected() {
    let workflow = read(".github/workflows/pre-commit-hooks.yml");
    let mutated = workflow.replace(
        "cargo run --quiet --locked",
        "uses: stefan-vatov/gh-workflows/.github/workflows/pre-commit-hooks.yml@main\n          cargo run --quiet --locked",
    );
    let violations = workflow_violations(&mutated);
    assert!(
        violations.contains(&"mutable or external reusable workflow authority"),
        "mutation must remain visibly rejected: {violations:?}"
    );
}

#[test]
fn writable_permissions_mutation_is_rejected() {
    let workflow = read(".github/workflows/pre-commit-hooks.yml");
    let mutated = workflow.replace(
        "permissions:\n  contents: read",
        "permissions:\n  contents: write",
    );
    let violations = workflow_violations(&mutated);
    assert!(
        violations.contains(&"workflow permissions are not bounded to contents: read"),
        "permission mutation must remain visibly rejected: {violations:?}"
    );
}

#[test]
fn workflow_consumers_select_manifest_owned_profiles() {
    let stable = read(".github/workflows/pr-lint-test.yml");
    let hooks = read(".github/workflows/pre-commit-hooks.yml");
    let coverage = read(".github/workflows/coverage.yml");
    assert!(stable.contains("--profile stable --json"));
    assert!(stable.contains("--profile msrv --json"));
    assert!(hooks.contains("--profile stable --json"));
    assert!(coverage.contains("--profile coverage --json"));
    assert!(!hooks.contains("validate_beads_decisions:"));
}

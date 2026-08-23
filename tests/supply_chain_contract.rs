//! Supply-chain and workflow contract tests.
//!
//! These tests use small in-memory fixtures so a mutation reports the exact
//! field and replay label that caused the failure.  The checks are deliberately
//! independent of GitHub's parser and of the private developer tool.

use std::{collections::BTreeSet, fs, path::PathBuf};

use serde::Deserialize;
use yaml_serde::{Mapping, Value};

const REQUIRED_GATES: &[&str] = &[
    "fmt",
    "clippy",
    "tests",
    "doctests",
    "build",
    "prek",
    "beads-validate",
    "beads-validator-tests",
    "beads-plan",
    "beads-plan-tests",
    "coverage",
    "msrv-tests",
    "msrv-doctests",
];

const WORKFLOW_MUTABLE_REF: &str = r#"
name: mutable
on: {workflow_dispatch: {}}
permissions: {contents: read}
jobs:
  publish:
    uses: acme/release/.github/workflows/publish.yml@main
"#;

const WORKFLOW_MASKED_STATUS: &str = r#"
name: masked
on: {pull_request: {}}
permissions: {contents: read}
jobs:
  quality:
    runs-on: ubuntu-latest
    steps:
      - run: cargo test --workspace --locked
        continue-on-error: true
"#;

const WORKFLOW_SAFE: &str = r#"
name: safe
on: {pull_request: {}}
permissions: {contents: read}
jobs:
  quality:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@0123456789abcdef0123456789abcdef01234567
      - run: cargo test --workspace --all-targets --all-features --locked
"#;

#[derive(Clone, Debug, Deserialize)]
struct Manifest {
    gates: Vec<Gate>,
    lockfiles: Lockfiles,
}

#[derive(Clone, Debug, Deserialize)]
struct Gate {
    id: String,
    toolchain: String,
    argv: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct Lockfiles {
    cargo_lock: CargoLock,
}

#[derive(Clone, Debug, Deserialize)]
struct CargoLock {
    tracked: bool,
    packaged: bool,
    validation_flag: String,
    update_behavior: String,
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn mapping_value<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a Value> {
    mapping.iter().find_map(|(candidate, value)| {
        if candidate.as_str() == Some(key) || (key == "on" && candidate.as_bool() == Some(true)) {
            Some(value)
        } else {
            None
        }
    })
}

fn collect_strings(value: &Value, path: &str, output: &mut Vec<(String, String)>) {
    if let Some(string) = value.as_str() {
        output.push((path.to_owned(), string.to_owned()));
        return;
    }
    if let Some(boolean) = value.as_bool() {
        output.push((path.to_owned(), boolean.to_string()));
        return;
    }
    if let Some(sequence) = value.as_sequence() {
        for (index, item) in sequence.iter().enumerate() {
            collect_strings(item, &format!("{path}[{index}]"), output);
        }
        return;
    }
    if let Some(mapping) = value.as_mapping() {
        for (key, item) in mapping {
            let key = key.as_str().unwrap_or("<non-string-key>");
            collect_strings(item, &format!("{path}.{key}"), output);
        }
    }
}

fn workflow_contract(source: &str, path: &str) -> Vec<String> {
    let document = yaml_serde::from_str::<Value>(source)
        .unwrap_or_else(|error| panic!("fixture={path}; invalid YAML: {error}"));
    let root = document
        .as_mapping()
        .unwrap_or_else(|| panic!("fixture={path}; workflow root is not a mapping"));
    let mut failures = Vec::new();

    let permissions = mapping_value(root, "permissions").and_then(Value::as_mapping);
    if permissions
        .and_then(|mapping| mapping_value(mapping, "contents"))
        .and_then(Value::as_str)
        != Some("read")
    {
        failures.push(format!("{path}.permissions.contents: expected read"));
    }

    let mut strings = Vec::new();
    collect_strings(&document, path, &mut strings);
    for (field, value) in strings {
        if field.ends_with(".uses")
            && value.contains("/.github/workflows/")
            && ["@main", "@master", "@stable", "@latest"]
                .iter()
                .any(|suffix| value.ends_with(suffix))
        {
            failures.push(format!(
                "{field}: mutable reusable workflow ref {value}; pin an immutable 40-character revision"
            ));
        }
        if field.ends_with(".continue-on-error") && value == "true" {
            failures.push(format!(
                "{field}: masked status is forbidden; replay with continue-on-error=true"
            ));
        }
    }
    failures
}

fn manifest_contract(manifest: &Manifest) -> Vec<String> {
    let mut failures = Vec::new();
    let actual = manifest
        .gates
        .iter()
        .map(|gate| gate.id.as_str())
        .collect::<BTreeSet<_>>();
    for required in REQUIRED_GATES {
        if !actual.contains(required) {
            failures.push(format!(
                "gates[{required}]: required quality gate is omitted"
            ));
        }
    }
    for (index, gate) in manifest.gates.iter().enumerate() {
        let required_flags: &[&str] = match gate.id.as_str() {
            "clippy" => &["--workspace", "--all-targets", "--all-features", "--locked"],
            "tests" | "build" | "msrv-tests" => {
                &["--workspace", "--all-targets", "--all-features", "--locked"]
            }
            "doctests" | "msrv-doctests" => &["--workspace", "--doc", "--all-features", "--locked"],
            "beads-validate" | "beads-validator-tests" | "beads-plan" | "beads-plan-tests" => {
                &["--locked"]
            }
            _ => &[],
        };
        for required in required_flags {
            if !gate.argv.iter().any(|argument| argument == required) {
                failures.push(format!(
                    "gates[{index}].argv: {} is missing required flag {required}",
                    gate.id
                ));
            }
        }
        if matches!(gate.id.as_str(), "msrv-tests" | "msrv-doctests")
            && gate.toolchain != "rust-1.86.0"
        {
            failures.push(format!(
                "gates[{index}].toolchain: {} must use rust-1.86.0, got {}",
                gate.id, gate.toolchain
            ));
        }
    }
    let lock = &manifest.lockfiles.cargo_lock;
    if !lock.tracked {
        failures.push("lockfiles.cargo_lock.tracked: Cargo.lock must remain tracked".to_owned());
    }
    if !lock.packaged {
        failures.push("lockfiles.cargo_lock.packaged: Cargo.lock must remain packaged".to_owned());
    }
    if lock.validation_flag != "--locked" {
        failures.push(format!(
            "lockfiles.cargo_lock.validation_flag: expected --locked, got {}",
            lock.validation_flag
        ));
    }
    if lock.update_behavior != "fail" {
        failures.push(format!(
            "lockfiles.cargo_lock.update_behavior: expected fail, got {}",
            lock.update_behavior
        ));
    }
    failures
}

fn assert_red(label: &str, failures: &[String]) {
    assert!(
        !failures.is_empty(),
        "fixture={label}; expected RED but got GREEN"
    );
    assert!(
        failures.iter().any(|failure| failure.contains(label)),
        "fixture={label}; no replayable path in failures: {failures:?}"
    );
}

fn read_manifest() -> Manifest {
    let source = fs::read_to_string(root().join("scripts/quality-manifest.json"))
        .expect("quality manifest must be readable");
    yaml_serde::from_str(&source).expect("quality manifest must deserialize")
}

#[test]
fn safe_workflow_fixture_is_green() {
    let failures = workflow_contract(WORKFLOW_SAFE, "fixture://safe.yml");
    assert!(
        failures.is_empty(),
        "fixture=safe; unexpected RED: {failures:?}"
    );
}

#[test]
fn mutable_control_ref_fixture_is_rejected_with_path() {
    let failures = workflow_contract(WORKFLOW_MUTABLE_REF, "mutable-control-ref");
    assert_red("mutable-control-ref.jobs.publish.uses", &failures);
}

#[test]
fn masked_status_fixture_is_rejected_with_path() {
    let failures = workflow_contract(WORKFLOW_MASKED_STATUS, "masked-status");
    assert_red(
        "masked-status.jobs.quality.steps[0].continue-on-error",
        &failures,
    );
}

#[test]
fn canonical_workflows_have_bounded_permissions_and_refs() {
    let directory = root().join(".github/workflows");
    let mut paths = fs::read_dir(&directory)
        .expect("workflow directory must exist")
        .map(|entry| entry.expect("workflow entry must be readable").path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("yml" | "yaml")
            )
        })
        .collect::<Vec<_>>();
    paths.sort();
    let failures = paths
        .iter()
        .flat_map(|path| {
            let source = fs::read_to_string(path).expect("workflow must be readable");
            workflow_contract(&source, &path.display().to_string())
        })
        .collect::<Vec<_>>();
    assert!(
        failures.is_empty(),
        "canonical workflows contain supply-chain violations with replayable paths: {failures:?}"
    );
}

#[test]
fn manifest_is_green_and_contains_every_required_gate() {
    let manifest = read_manifest();
    let failures = manifest_contract(&manifest);
    assert!(
        failures.is_empty(),
        "canonical quality manifest is RED: {failures:?}"
    );
}

#[test]
fn omitted_beads_prek_or_coverage_gate_is_rejected() {
    let mut manifest = read_manifest();
    for omitted in ["beads-plan-tests", "prek", "coverage"] {
        manifest.gates.retain(|gate| gate.id != omitted);
        let failures = manifest_contract(&manifest);
        assert_red(&format!("gates[{omitted}]"), &failures);
        manifest = read_manifest();
    }
}

#[test]
fn missing_locked_flag_is_rejected_with_gate_path() {
    let mut manifest = read_manifest();
    let gate = manifest
        .gates
        .iter_mut()
        .find(|gate| gate.id == "build")
        .expect("build gate exists");
    gate.argv.retain(|argument| argument != "--locked");
    let failures = manifest_contract(&manifest);
    assert_red("gates[4].argv", &failures);
}

#[test]
fn altered_msrv_is_rejected_with_gate_path() {
    let mut manifest = read_manifest();
    let gate = manifest
        .gates
        .iter_mut()
        .find(|gate| gate.id == "msrv-tests")
        .expect("msrv-tests gate exists");
    gate.toolchain = "stable".to_owned();
    let failures = manifest_contract(&manifest);
    assert_red("gates[11].toolchain", &failures);
}

#[test]
fn ignored_or_unpackaged_cargo_lock_is_rejected() {
    let mut manifest = read_manifest();
    manifest.lockfiles.cargo_lock.tracked = false;
    let failures = manifest_contract(&manifest);
    assert_red("lockfiles.cargo_lock.tracked", &failures);

    let gitignore = fs::read_to_string(root().join(".gitignore")).expect("read .gitignore");
    assert!(
        !gitignore.lines().any(|line| line.trim() == "Cargo.lock"),
        "path=.gitignore:Cargo.lock; Cargo.lock must not be ignored"
    );
    let cargo_manifest = fs::read_to_string(root().join("Cargo.toml")).expect("read Cargo.toml");
    assert!(
        !cargo_manifest
            .lines()
            .any(|line| line.trim() == "\"Cargo.lock\""),
        "path=Cargo.toml:package.exclude.Cargo.lock; Cargo.lock must remain packaged"
    );
}

#[test]
fn owner_package_contract_is_binary_only_and_excludes_private_surfaces() {
    let manifest = fs::read_to_string(root().join("Cargo.toml")).expect("read Cargo.toml");
    assert!(
        manifest.contains("rust-version = \"1.86\""),
        "path=Cargo.toml:rust-version"
    );
    assert!(
        !manifest.contains("\n[lib]"),
        "path=Cargo.toml:[lib]; product is binary-only"
    );
    for excluded in [
        "tools/omnirepo-dev",
        "tools/omnirepo-test-support",
        ".beads",
        "tests",
    ] {
        assert!(
            manifest.contains(excluded),
            "path=Cargo.toml:package.exclude; missing private surface {excluded}"
        );
    }
}

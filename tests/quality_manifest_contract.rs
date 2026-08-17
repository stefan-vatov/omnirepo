use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
struct Manifest {
    schema: String,
    version: u64,
    gates: Vec<Gate>,
    profiles: Vec<Profile>,
    aliases: Vec<Alias>,
    lockfiles: Lockfiles,
}

#[derive(Clone, Debug, Deserialize)]
struct Gate {
    id: String,
    kind: String,
    toolchain: String,
    working_directory: String,
    argv: Vec<String>,
    failure_identity: String,
    authority: String,
    owner: String,
}

#[derive(Clone, Debug, Deserialize)]
struct Profile {
    name: String,
    kind: String,
    gates: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct Alias {
    name: String,
    kind: String,
    canonical_gate: String,
    classification: String,
}

#[derive(Clone, Debug, Deserialize)]
struct Lockfiles {
    cargo_lock: CargoLock,
    package_lock: PackageLock,
}

#[derive(Clone, Debug, Deserialize)]
struct CargoLock {
    tracked: bool,
    packaged: bool,
    validation_flag: String,
    update_behavior: String,
}

#[derive(Clone, Debug, Deserialize)]
struct PackageLock {
    status: String,
    policy: String,
}

const EXPECTED_GATES: &[(&str, &str, &[&str])] = &[
    (
        "fmt",
        "rust-1.86.0",
        &["scripts/cargo-1.86", "fmt", "--all", "--", "--check"],
    ),
    (
        "clippy",
        "rust-1.86.0",
        &[
            "scripts/cargo-1.86",
            "clippy",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    ),
    (
        "tests",
        "rust-1.86.0",
        &[
            "scripts/cargo-1.86",
            "test",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--locked",
        ],
    ),
    (
        "doctests",
        "rust-1.86.0",
        &[
            "scripts/cargo-1.86",
            "test",
            "--workspace",
            "--doc",
            "--all-features",
            "--locked",
        ],
    ),
    (
        "build",
        "rust-1.86.0",
        &[
            "scripts/cargo-1.86",
            "build",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--locked",
        ],
    ),
    (
        "pre-commit",
        "system",
        &["pre-commit", "run", "--all-files"],
    ),
    (
        "beads-validate",
        "rust-1.86.0",
        &[
            "scripts/cargo-1.86",
            "run",
            "--quiet",
            "--locked",
            "--manifest-path",
            "tools/omnirepo-dev/Cargo.toml",
            "--",
            "validate-decisions",
        ],
    ),
    (
        "beads-validator-tests",
        "rust-1.86.0",
        &[
            "scripts/cargo-1.86",
            "test",
            "--quiet",
            "--locked",
            "--manifest-path",
            "tools/omnirepo-dev/Cargo.toml",
            "--test",
            "beads_validator_contract",
        ],
    ),
    (
        "beads-plan",
        "rust-1.86.0",
        &[
            "scripts/cargo-1.86",
            "run",
            "--quiet",
            "--locked",
            "--manifest-path",
            "tools/omnirepo-dev/Cargo.toml",
            "--",
            "plan",
            "--repo-root",
            ".",
            "--json",
        ],
    ),
    (
        "beads-plan-tests",
        "rust-1.86.0",
        &[
            "scripts/cargo-1.86",
            "test",
            "--quiet",
            "--locked",
            "--manifest-path",
            "tools/omnirepo-dev/Cargo.toml",
            "--test",
            "planner_contract",
        ],
    ),
    (
        "coverage",
        "rust-1.86.0/cargo-llvm-cov-0.8.7",
        &["bash", "scripts/coverage.sh"],
    ),
    (
        "msrv-tests",
        "rust-1.86.0",
        &[
            "scripts/cargo-1.86",
            "test",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--locked",
        ],
    ),
    (
        "msrv-doctests",
        "rust-1.86.0",
        &[
            "scripts/cargo-1.86",
            "test",
            "--workspace",
            "--doc",
            "--all-features",
            "--locked",
        ],
    ),
];

fn manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/quality-manifest.json")
}

fn read_manifest() -> (Manifest, String) {
    let path = manifest_path();
    let source = fs::read_to_string(&path).expect("quality manifest must be readable");
    let manifest = yaml_serde::from_str(&source).expect("quality manifest must be valid JSON");
    (manifest, source)
}

fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
    if manifest.schema != "omnirepo.quality-manifest.v1" || manifest.version != 1 {
        return Err("manifest schema or version changed".to_owned());
    }

    if manifest.gates.len() != EXPECTED_GATES.len() {
        return Err(format!(
            "expected {} gates, found {}",
            EXPECTED_GATES.len(),
            manifest.gates.len()
        ));
    }

    let gate_ids = manifest
        .gates
        .iter()
        .map(|gate| gate.id.as_str())
        .collect::<Vec<_>>();
    let unique_gate_ids = gate_ids.iter().copied().collect::<HashSet<_>>();
    if unique_gate_ids.len() != gate_ids.len() {
        return Err("gate IDs are duplicated".to_owned());
    }

    for ((expected_id, expected_toolchain, expected_argv), gate) in
        EXPECTED_GATES.iter().zip(&manifest.gates)
    {
        if gate.id != *expected_id
            || gate.toolchain != *expected_toolchain
            || gate.argv
                != expected_argv
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
        {
            return Err(format!(
                "gate {} has changed command or toolchain",
                expected_id
            ));
        }
        if gate.kind != "gate"
            || gate.working_directory != "."
            || gate.argv.is_empty()
            || gate.failure_identity.is_empty()
            || !gate.failure_identity.starts_with("quality.")
            || gate.authority.is_empty()
            || gate.owner.is_empty()
        {
            return Err(format!("gate {} has incomplete metadata", gate.id));
        }
    }

    let failure_identities = manifest
        .gates
        .iter()
        .map(|gate| gate.failure_identity.as_str())
        .collect::<Vec<_>>();
    if failure_identities
        .iter()
        .copied()
        .collect::<HashSet<_>>()
        .len()
        != failure_identities.len()
    {
        return Err("failure identities are duplicated".to_owned());
    }

    let coverage = manifest
        .gates
        .iter()
        .find(|gate| gate.id == "coverage")
        .ok_or_else(|| "coverage gate is missing".to_owned())?;
    if coverage.authority != "omni-constitutional-convergence-2r9.34"
        || coverage.owner != "omni-constitutional-convergence-2r9.34"
        || coverage.argv != ["bash", "scripts/coverage.sh"]
    {
        return Err("coverage gate does not delegate to .34".to_owned());
    }

    let gate_id_set = gate_ids.into_iter().collect::<HashSet<_>>();
    let profile_names = manifest
        .profiles
        .iter()
        .map(|profile| profile.name.as_str())
        .collect::<Vec<_>>();
    if profile_names.len() != 4
        || profile_names.iter().copied().collect::<HashSet<_>>().len() != profile_names.len()
        || !["full", "stable", "msrv", "coverage"]
            .iter()
            .all(|required| profile_names.contains(required))
    {
        return Err("required quality profiles are missing or duplicated".to_owned());
    }
    for profile in &manifest.profiles {
        if profile.kind != "profile"
            || profile.name.is_empty()
            || profile.gates.is_empty()
            || profile.gates.iter().collect::<HashSet<_>>().len() != profile.gates.len()
            || profile
                .gates
                .iter()
                .any(|gate| !gate_id_set.contains(gate.as_str()))
        {
            return Err(format!("profile {} is invalid", profile.name));
        }
    }
    let full = manifest
        .profiles
        .iter()
        .find(|profile| profile.name == "full")
        .expect("full profile must exist");
    if full.gates.len() != manifest.gates.len()
        || full
            .gates
            .iter()
            .any(|gate| !gate_id_set.contains(gate.as_str()))
    {
        return Err("full profile must select every gate exactly once".to_owned());
    }
    let alias_names = manifest
        .aliases
        .iter()
        .map(|alias| alias.name.as_str())
        .collect::<Vec<_>>();
    if alias_names.iter().copied().collect::<HashSet<_>>().len() != alias_names.len() {
        return Err("aliases are duplicated".to_owned());
    }
    for alias in &manifest.aliases {
        if alias.kind != "alias"
            || alias.classification.is_empty()
            || !gate_id_set.contains(alias.canonical_gate.as_str())
            || alias.name == "coverage"
        {
            return Err(format!("alias {} is not classified", alias.name));
        }
    }

    if !manifest.lockfiles.cargo_lock.tracked
        || !manifest.lockfiles.cargo_lock.packaged
        || manifest.lockfiles.cargo_lock.validation_flag != "--locked"
        || manifest.lockfiles.cargo_lock.update_behavior != "fail"
        || manifest.lockfiles.package_lock.status != "not-used"
        || manifest.lockfiles.package_lock.policy.is_empty()
    {
        return Err("lockfile policy is incomplete".to_owned());
    }

    Ok(())
}

#[test]
fn canonical_manifest_is_complete_and_exact() {
    let (manifest, source) = read_manifest();
    validate_manifest(&manifest).expect("quality manifest contract must pass");
    assert!(!source.contains("fail-under"));
    assert!(!source.contains("COVERAGE_LINES_MIN"));
    assert!(!source.contains("COVERAGE_FUNCTIONS_MIN"));
    assert!(!source.contains("COVERAGE_REGIONS_MIN"));
}

#[test]
fn omitted_gate_is_rejected() {
    let (mut manifest, _) = read_manifest();
    manifest.gates.remove(1);
    assert!(validate_manifest(&manifest).is_err());
}

#[test]
fn duplicated_gate_is_rejected() {
    let (mut manifest, _) = read_manifest();
    let duplicate = manifest.gates[0].clone();
    manifest.gates[1] = duplicate;
    assert!(validate_manifest(&manifest).is_err());
}

#[test]
fn changed_command_is_rejected() {
    let (mut manifest, _) = read_manifest();
    manifest.gates[0].argv.push("--unexpected".to_owned());
    assert!(validate_manifest(&manifest).is_err());
}

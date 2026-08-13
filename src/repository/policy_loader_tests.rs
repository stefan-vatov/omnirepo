//! Focused proof for canonical root repository policy loading.

#![allow(dead_code, unused_imports)]

use super::policy::{CommandPolicy, RepositoryPolicy, SelectionPolicy};
use super::policy_loader::{
    COMPETING_FILE_NAME, LEGACY_FILE_NAME, POLICY_FILE_NAME, PolicyLoadError, PolicyPresence,
    load_policy,
};
use std::{fs, path::Path};

fn fixture_root() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("create filesystem fixture base");
    tempfile::Builder::new()
        .prefix("policy-home-")
        .tempdir_in(&base)
        .expect("create policy fixture")
}

fn write_policy(root: &Path, content: &str) {
    fs::write(root.join(POLICY_FILE_NAME), content).expect("write policy");
}

const VALID_POLICY: &str = "version: 1
all: true
allow: [docs, scripts]
exclude: [scripts]
commands:
  - [verify, --check]
  - [git, status]
";

#[test]
fn absent_policy_is_a_distinct_lawful_state() {
    let root = fixture_root();
    assert_eq!(
        load_policy(root.path()).expect("load"),
        PolicyPresence::Absent
    );
}

#[test]
fn valid_policy_loads_exact_typed_values() {
    let root = fixture_root();
    write_policy(root.path(), VALID_POLICY);
    let PolicyPresence::Present(policy) = load_policy(root.path()).expect("load") else {
        panic!("expected present policy");
    };
    assert_eq!(policy.schema_version().value(), 1);
    let selection = policy.selection();
    assert!(!selection.is_omitted());
    assert!(selection.all());
    assert_eq!(selection.allow().len(), 2);
    assert_eq!(selection.exclude().len(), 1);
    let commands = policy.commands();
    assert!(commands.is_present());
    let argv = commands.as_slice().expect("commands");
    assert_eq!(argv.len(), 2);
    assert_eq!(argv[0].argv(), &["verify", "--check"]);
}

#[test]
fn omitted_selectors_select_nothing_without_inference() {
    let root = fixture_root();
    write_policy(root.path(), "version: 1\n");
    let PolicyPresence::Present(policy) = load_policy(root.path()).expect("load") else {
        panic!("expected present policy");
    };
    assert!(policy.selection().is_omitted());
    assert!(policy.selection().selects_nothing());
    assert!(policy.commands().is_absent());
}

#[test]
fn commands_only_policy_is_present_and_selects_nothing() {
    let root = fixture_root();
    write_policy(root.path(), "version: 1\ncommands:\n  - [verify]\n");
    let PolicyPresence::Present(policy) = load_policy(root.path()).expect("load") else {
        panic!("expected present policy");
    };
    assert!(policy.commands().is_present());
    assert!(policy.selection().selects_nothing());
}

#[test]
fn symlink_policy_is_a_typed_alias_error() {
    let root = fixture_root();
    let real = root.path().join("real.yaml");
    fs::write(&real, VALID_POLICY).expect("write real");
    std::os::unix::fs::symlink(&real, root.path().join(POLICY_FILE_NAME)).expect("symlink");
    let error = load_policy(root.path()).expect_err("symlink must fail");
    assert!(matches!(error, PolicyLoadError::Alias { .. }), "{error:?}");
}

#[test]
fn directory_policy_is_a_typed_non_regular_error() {
    let root = fixture_root();
    fs::create_dir_all(root.path().join(POLICY_FILE_NAME)).expect("create dir");
    let error = load_policy(root.path()).expect_err("directory must fail");
    assert!(
        matches!(error, PolicyLoadError::NotRegular { .. }),
        "{error:?}"
    );
}

#[test]
fn competing_extension_is_a_typed_error() {
    let root = fixture_root();
    write_policy(root.path(), VALID_POLICY);
    fs::write(root.path().join(COMPETING_FILE_NAME), "version: 1\n").expect("competing");
    let error = load_policy(root.path()).expect_err("competing must fail");
    assert!(
        matches!(error, PolicyLoadError::Competing { .. }),
        "{error:?}"
    );
}

#[test]
fn legacy_authority_is_an_error_not_a_fallback() {
    let root = fixture_root();
    fs::write(root.path().join(LEGACY_FILE_NAME), "legacy\n").expect("legacy");
    let error = load_policy(root.path()).expect_err("legacy must fail");
    assert!(
        matches!(error, PolicyLoadError::LegacyAuthority { .. }),
        "{error:?}"
    );
}

#[test]
fn malformed_and_unknown_fields_fail_closed() {
    let root = fixture_root();
    write_policy(root.path(), "version: 1\nallow: [unclosed\n");
    let error = load_policy(root.path()).expect_err("malformed must fail");
    assert!(
        matches!(error, PolicyLoadError::Malformed { .. }),
        "{error:?}"
    );

    fs::write(
        root.path().join(POLICY_FILE_NAME),
        "version: 1\nfleet: [x]\n",
    )
    .expect("write");
    let error = load_policy(root.path()).expect_err("unknown field must fail");
    assert!(
        matches!(error, PolicyLoadError::Malformed { .. }),
        "{error:?}"
    );
}

#[test]
fn unsupported_version_is_a_typed_error() {
    let root = fixture_root();
    write_policy(root.path(), "version: 2\n");
    let error = load_policy(root.path()).expect_err("unsupported version must fail");
    assert!(
        matches!(
            error,
            PolicyLoadError::UnsupportedVersion { version: 2, .. }
        ),
        "{error:?}"
    );
}

#[test]
fn duplicate_and_invalid_policy_values_fail_closed() {
    let root = fixture_root();
    fs::write(
        root.path().join(POLICY_FILE_NAME),
        "version: 1\nallow: [docs, docs]\n",
    )
    .expect("write");
    let error = load_policy(root.path()).expect_err("duplicate allow must fail");
    assert!(
        matches!(error, PolicyLoadError::Invalid { .. }),
        "{error:?}"
    );

    fs::write(
        root.path().join(POLICY_FILE_NAME),
        "version: 1\ncommands:\n  - [\"\"]\n",
    )
    .expect("write");
    let error = load_policy(root.path()).expect_err("empty executable must fail");
    assert!(
        matches!(error, PolicyLoadError::Invalid { .. }),
        "{error:?}"
    );
}

#[test]
fn repository_policy_never_grants_fleet_or_source_authority() {
    let root = fixture_root();
    write_policy(root.path(), VALID_POLICY);
    let PolicyPresence::Present(policy) = load_policy(root.path()).expect("load") else {
        panic!("expected present policy");
    };
    // The policy surface carries only schema, selection, and commands; no
    // fleet, source, or priority concept appears in its representation.
    let rendered = format!("{policy:?}");
    assert!(!rendered.contains("fleet"), "{rendered}");
    assert!(!rendered.contains("source"), "{rendered}");
    assert!(!rendered.contains("priority"), "{rendered}");
}

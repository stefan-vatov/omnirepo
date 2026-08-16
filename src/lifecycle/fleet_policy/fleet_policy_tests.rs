//! Focused proof for per-destination repository policy loading with
//! lawful absence.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::configuration::{
    AbsolutePath, DestinationRepository, MachineConcurrency, MachineConfiguration, RepairControls,
    RepositoryId, SchemaVersion,
};
use crate::lifecycle::fleet_policy::load_repository_policies;
use crate::lifecycle::plan_selection::Policy;
use std::{fs, path::Path, process::Command};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("fleet-policy-")
        .tempdir_in(&base)
        .expect("fixture")
}

fn destination(root: &Path, id: &str) -> DestinationRepository {
    let path = root.join(id);
    fs::create_dir_all(&path).expect("destination");
    DestinationRepository::new(
        RepositoryId::parse(id).expect("repository id"),
        AbsolutePath::parse(path.to_str().expect("utf8")).expect("path"),
        Vec::new(),
    )
    .expect("destination")
}

fn machine(repositories: Vec<DestinationRepository>) -> MachineConfiguration {
    MachineConfiguration::new(
        SchemaVersion::new(1).expect("version"),
        repositories,
        Vec::new(),
        None,
        MachineConcurrency::new(4, 8).expect("concurrency"),
        RepairControls::default(),
    )
    .expect("machine")
}

fn write_policy(root: &Path, id: &str, content: &str) {
    fs::write(root.join(id).join(".omnirepo.yaml"), content).expect("policy");
}

#[test]
fn absent_policy_is_lawful_and_inference_governs() {
    let fixture = fixture_base();
    let config = machine(vec![destination(fixture.path(), "repo-a")]);
    let loads = load_repository_policies(&config);
    assert_eq!(loads.len(), 1);
    assert_eq!(loads[0].repository, "repo-a");
    assert!(loads[0].failure.is_none(), "{:?}", loads[0]);
    // Lawful absence: no policy at all -> the absent plan policy, which
    // triggers inference downstream.
    assert_eq!(loads[0].policy, Some(Policy::Absent));
}

#[test]
fn explicit_policy_converts_to_the_plan_policy_exactly() {
    let fixture = fixture_base();
    destination(fixture.path(), "repo-a");
    write_policy(
        fixture.path(),
        "repo-a",
        "version: 1\nallow:\n  - item-1\n  - item-2\nexclude:\n  - item-2\n",
    );
    let config = machine(vec![destination(fixture.path(), "repo-a")]);
    let loads = load_repository_policies(&config);
    assert_eq!(
        loads[0].policy,
        Some(Policy::Explicit {
            include: vec!["item-1".to_owned(), "item-2".to_owned()],
            exclude: vec!["item-2".to_owned()],
        }),
        "exclusion wins is decided downstream by the selection table"
    );
}

#[test]
fn omitted_selectors_select_nothing_and_never_infer() {
    let fixture = fixture_base();
    destination(fixture.path(), "repo-a");
    write_policy(
        fixture.path(),
        "repo-a",
        "version: 1\ncommands:\n  - [echo, ok]\n",
    );
    let config = machine(vec![destination(fixture.path(), "repo-a")]);
    let loads = load_repository_policies(&config);
    // Present policy with omitted selectors: explicit empty selection,
    // never the absent (inference) policy.
    assert_eq!(
        loads[0].policy,
        Some(Policy::Explicit {
            include: Vec::new(),
            exclude: Vec::new(),
        }),
        "load failure: {:?}",
        loads[0].failure
    );
}

#[test]
fn a_policy_failure_fails_only_that_destination() {
    let fixture = fixture_base();
    // repo-a: a symlink alias policy (refused before content reads).
    destination(fixture.path(), "repo-a");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            fixture.path().join("outside.yaml"),
            fixture.path().join("repo-a/.omnirepo.yaml"),
        )
        .expect("alias");
    }
    fs::write(fixture.path().join("outside.yaml"), "version: 1\n").expect("outside");
    // repo-b: a valid explicit policy.
    destination(fixture.path(), "repo-b");
    write_policy(fixture.path(), "repo-b", "version: 1\nall: true\n");
    let config = machine(vec![
        destination(fixture.path(), "repo-a"),
        destination(fixture.path(), "repo-b"),
    ]);
    let loads = load_repository_policies(&config);
    assert_eq!(loads.len(), 2, "both destinations are accounted");
    assert!(
        loads[0].failure.is_some(),
        "repo-a fails typed: {:?}",
        loads[0]
    );
    assert_eq!(loads[0].policy, None, "a failed load carries no policy");
    assert!(
        loads[1].failure.is_none(),
        "repo-b is unaffected: {:?}",
        loads[1]
    );
    assert_eq!(
        loads[1].policy,
        Some(Policy::Explicit {
            include: Vec::new(),
            exclude: Vec::new(),
        })
    );
}

#[test]
fn declared_order_is_preserved_across_all_loads() {
    let fixture = fixture_base();
    let config = machine(vec![
        destination(fixture.path(), "repo-zeta"),
        destination(fixture.path(), "repo-alpha"),
        destination(fixture.path(), "repo-mid"),
    ]);
    let loads = load_repository_policies(&config);
    let ids = loads
        .iter()
        .map(|load| load.repository.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["repo-zeta", "repo-alpha", "repo-mid"]);
}

#[test]
fn declared_commands_are_carried_into_the_fleet_pass() {
    let fixture = fixture_base();
    destination(fixture.path(), "repo-a");
    write_policy(
        fixture.path(),
        "repo-a",
        "version: 1\ncommands:\n  - [echo, ok]\n  - [/bin/true]\n",
    );
    let config = machine(vec![destination(fixture.path(), "repo-a")]);
    let loads = load_repository_policies(&config);
    let checks = &loads[0].checks;
    assert_eq!(
        checks.len(),
        2,
        "commands survive the policy load: {:?}",
        loads[0]
    );
    assert_eq!(checks[0].argv(), &["echo".to_owned(), "ok".to_owned()]);
    assert_eq!(checks[1].argv(), &["/bin/true".to_owned()]);
    // An absent policy carries no commands.
    let absent = fixture_base();
    destination(absent.path(), "repo-b");
    let config = machine(vec![destination(absent.path(), "repo-b")]);
    let loads = load_repository_policies(&config);
    assert!(loads[0].checks.is_empty(), "{:?}", loads[0]);
}

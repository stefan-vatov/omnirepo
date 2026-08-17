//! Focused proof for running one repository initial pass per admitted
//! item through the fleet runner.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::configuration::{
    AbsolutePath, DestinationRepository, MachineConcurrency, MachineConfiguration, RepairControls,
    RepositoryId, SchemaVersion,
};
use crate::lifecycle::fleet_app::compose_fleet;
use crate::lifecycle::fleet_composition::compose_configured_fleet;
use crate::lifecycle::fleet_planning::{RepositoryPlan, build_repository_plans};
use crate::lifecycle::fleet_runner::run_fleet_initial_passes;
use crate::lifecycle::journal::{Journal, JournalConfig};
use crate::lifecycle::run_record::RunRecord;
use crate::lifecycle::sync_plan::PlanDecision;
use crate::lifecycle::sync_plan::{PlanItem, SyncPlan};
use crate::lifecycle::work_mapping::WorkItem;
use crate::source::{CatalogState, ItemKind, RevisionId, SourceCatalog, SourceId};
use std::{
    fs,
    path::Path,
    process::Command,
    time::{Duration, SystemTime},
};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("fleet-runner-")
        .tempdir_in(&base)
        .expect("fixture")
}

fn git_repo(root: &Path) {
    fs::create_dir_all(root).expect("repo");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["init", "--quiet", "-b", "main"]);
    git(&["config", "user.name", "Runner"]);
    git(&["config", "user.email", "runner@example.test"]);
    fs::write(root.join("managed.txt"), "v1\n").expect("file");
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "base"]);
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
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/fleet-runner-sources");
    fs::create_dir_all(base.join("source-a")).expect("source dir");
    fs::write(base.join("source-a/managed.txt"), "v1\n").expect("source file");
    let source = crate::configuration::SourceReference::new(
        crate::configuration::SourceId::parse("source-a").expect("source id"),
        crate::configuration::SourceLocation::local(
            crate::configuration::AbsolutePath::parse(
                base.join("source-a").to_str().expect("utf8"),
            )
            .expect("source path"),
        ),
    );
    MachineConfiguration::new(
        SchemaVersion::new(1).expect("version"),
        repositories,
        vec![source],
        None,
        MachineConcurrency::new(4, 8).expect("concurrency"),
        RepairControls::default(),
    )
    .expect("machine")
}

fn complete_catalog(source: &str) -> SourceCatalog {
    let mut catalog = SourceCatalog::new();
    catalog
        .record(CatalogState::Complete {
            source: SourceId::new(source).expect("source"),
            revision: RevisionId::new("rev-1").expect("revision"),
        })
        .expect("record");
    catalog
}

fn plan_item(id: &str, target: &str) -> PlanItem {
    PlanItem {
        id: id.to_owned(),
        target: target.to_owned(),
        source: "source-a".to_owned(),
        source_path: "managed.txt".to_owned(),
        source_order: 0,
        kind: ItemKind::WholeFile,
        decision: PlanDecision::Selected {
            reason: "inferred".to_owned(),
        },
    }
}

fn journal_fixture() -> (tempfile::TempDir, Journal, String) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("fleet-runner-home-")
        .tempdir_in(&base)
        .expect("fixture");
    fs::create_dir_all(fixture.path().join(".omnirepo/runs")).expect("runs");
    let record = RunRecord::create_with_id(
        fixture.path(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        [7_u8; 16],
    )
    .expect("record");
    let run_id = record.id().to_string();
    let journal = Journal::start(record, JournalConfig::default());
    (fixture, journal, run_id)
}

#[test]
fn every_admitted_repository_reaches_exactly_one_result_in_declared_order() {
    let fixture = fixture_base();
    let repo_a = fixture.path().join("repo-a");
    let repo_b = fixture.path().join("repo-b");
    git_repo(&repo_a);
    git_repo(&repo_b);
    let config = machine(vec![
        destination(fixture.path(), "repo-a"),
        destination(fixture.path(), "repo-b"),
    ]);
    let catalog = complete_catalog("source-a");
    let plans = vec![
        RepositoryPlan {
            repository: "repo-a".to_owned(),
            plan: Ok(SyncPlan::new(
                "repo-a",
                vec![plan_item("item-1", "managed.txt")],
            )),
            checks: Vec::new(),
        },
        RepositoryPlan {
            repository: "repo-b".to_owned(),
            plan: Ok(SyncPlan::new(
                "repo-b",
                vec![plan_item("item-2", "managed.txt")],
            )),
            checks: Vec::new(),
        },
    ];
    let outcome = compose_configured_fleet(&config, &catalog, &plans, None).expect("compose");
    let (_jfixture, mut journal, run_id) = journal_fixture();
    let response = run_fleet_initial_passes(
        &journal.handle,
        &run_id,
        &config,
        &plans,
        &outcome.composition,
        4,
    )
    .expect("pass");
    assert_eq!(response.results.len(), 2, "one outcome per repo");
    let ids = response
        .results
        .iter()
        .map(|result| match result {
            crate::lifecycle::fleet_collector::MemberResult::Delivered { repository, .. }
            | crate::lifecycle::fleet_collector::MemberResult::Failed { repository, .. }
            | crate::lifecycle::fleet_collector::MemberResult::Skipped { repository, .. } => {
                repository.as_str()
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["repo-a", "repo-b"], "declared order preserved");
    assert!(
        response.results.iter().all(|result| matches!(
            result,
            crate::lifecycle::fleet_collector::MemberResult::Delivered { .. }
        )),
        "{:?}",
        response.results
    );
    journal.shutdown().expect("shutdown");
}

#[test]
fn a_failing_repository_never_stops_its_peers() {
    let fixture = fixture_base();
    let good = fixture.path().join("repo-good");
    git_repo(&good);
    // repo-bad: a destination that is not a git repository.
    let bad = fixture.path().join("repo-bad");
    fs::create_dir_all(&bad).expect("bad destination");
    let config = machine(vec![
        destination(fixture.path(), "repo-good"),
        destination(fixture.path(), "repo-bad"),
    ]);
    let catalog = complete_catalog("source-a");
    let plans = vec![
        RepositoryPlan {
            repository: "repo-good".to_owned(),
            plan: Ok(SyncPlan::new(
                "repo-good",
                vec![plan_item("item-1", "managed.txt")],
            )),
            checks: Vec::new(),
        },
        RepositoryPlan {
            repository: "repo-bad".to_owned(),
            plan: Ok(SyncPlan::new(
                "repo-bad",
                vec![plan_item("item-2", "managed.txt")],
            )),
            checks: Vec::new(),
        },
    ];
    let outcome = compose_configured_fleet(&config, &catalog, &plans, None).expect("compose");
    let (_jfixture, mut journal, run_id) = journal_fixture();
    let response = run_fleet_initial_passes(
        &journal.handle,
        &run_id,
        &config,
        &plans,
        &outcome.composition,
        4,
    )
    .expect("pass");
    assert_eq!(response.results.len(), 2, "both repos are accounted");
    let good_result = response
        .results
        .iter()
        .find(|result| matches!(result, crate::lifecycle::fleet_collector::MemberResult::Delivered { repository, .. } if repository == "repo-good"))
        .expect("repo-good succeeds");
    let _ = good_result;
    let bad_failed = response
        .results
        .iter()
        .any(|result| matches!(result, crate::lifecycle::fleet_collector::MemberResult::Failed { repository, .. } if repository == "repo-bad"));
    assert!(
        bad_failed,
        "repo-bad fails typed without stopping repo-good"
    );
    journal.shutdown().expect("shutdown");
}

#[test]
fn an_unchanged_repository_creates_no_commit() {
    let fixture = fixture_base();
    let repo = fixture.path().join("repo-a");
    git_repo(&repo);
    let head_before = git_text(&repo, &["rev-parse", "HEAD"]);
    let config = machine(vec![destination(fixture.path(), "repo-a")]);
    let catalog = complete_catalog("source-a");
    let plans = vec![RepositoryPlan {
        repository: "repo-a".to_owned(),
        plan: Ok(SyncPlan::new(
            "repo-a",
            vec![plan_item("item-1", "managed.txt")],
        )),
        checks: Vec::new(),
    }];
    let outcome = compose_configured_fleet(&config, &catalog, &plans, None).expect("compose");
    let (_jfixture, mut journal, run_id) = journal_fixture();
    run_fleet_initial_passes(
        &journal.handle,
        &run_id,
        &config,
        &plans,
        &outcome.composition,
        4,
    )
    .expect("pass");
    let head_after = git_text(&repo, &["rev-parse", "HEAD"]);
    assert_eq!(
        head_before, head_after,
        "an unchanged repository creates no commit"
    );
    journal.shutdown().expect("shutdown");
}

fn git_text(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .current_dir(root)
        .output()
        .expect("git");
    assert!(output.status.success(), "git {args:?}: {:?}", output);
    String::from_utf8(output.stdout)
        .expect("stdout")
        .trim()
        .to_owned()
}

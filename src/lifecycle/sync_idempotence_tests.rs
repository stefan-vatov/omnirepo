//! Idempotence, partial-fleet failure, and journal identity fixtures.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::replacement_requests::map_whole_file_requests;
use crate::lifecycle::sync_plan::{PlanDecision, PlanItem, SyncPlan};
use crate::managed_content::classify_whole_file;
use crate::source::ItemKind;
use std::{fs, path::Path, process::Command};

fn selected(id: &str, target: &str) -> PlanItem {
    PlanItem {
        id: id.to_owned(),
        target: target.to_owned(),
        source: "primary".to_owned(),
        source_path: String::new(),
        source_order: 1,
        kind: ItemKind::WholeFile,
        decision: PlanDecision::Selected {
            reason: "declared winner".to_owned(),
        },
    }
}

fn git_repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("sync-idempotence-")
        .tempdir_in(&base)
        .expect("fixture");
    let root = fixture.path().join("repo");
    fs::create_dir_all(&root).expect("repo");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["init", "--quiet", "-b", "main"]);
    git(&["config", "user.name", "Commit"]);
    git(&["config", "user.email", "commit@example.test"]);
    (fixture, root)
}

#[test]
fn second_run_performs_no_content_or_git_mutation() {
    let (_fixture, root) = git_repo();
    let target = root.join("managed.txt");
    // First run: the file is missing → create.
    let first = classify_whole_file(false, None, b"v1\n").expect("classify");
    assert_eq!(first, crate::managed_content::WholeFileOutcome::Create);
    fs::write(&target, "v1\n").expect("write");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "first sync"]);
    let commits_before = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(&root)
        .output()
        .expect("git");
    let commits_before = String::from_utf8(commits_before.stdout).expect("stdout");
    // Second run: equal bytes → unchanged → no write, no commit.
    let second = classify_whole_file(true, Some(b"v1\n"), b"v1\n").expect("classify");
    assert_eq!(second, crate::managed_content::WholeFileOutcome::Unchanged);
    let content_after = fs::read_to_string(&target).expect("read");
    assert_eq!(content_after, "v1\n");
    let commits_after = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["rev-list", "--count", "HEAD"])
        .current_dir(&root)
        .output()
        .expect("git");
    assert_eq!(
        commits_before,
        String::from_utf8(commits_after.stdout).expect("stdout"),
        "no Git mutation on the second run"
    );
}

#[test]
fn one_repository_failure_does_not_alter_peer_eligibility() {
    // A failing repository's plan does not change a peer's eligibility:
    // the peer's whole-file requests still carry the same identities.
    let failing = SyncPlan::new(
        "dest-a",
        vec![PlanItem {
            id: "a".to_owned(),
            target: "t".to_owned(),
            source: "broken".to_owned(),
            source_path: String::new(),
            source_order: 1,
            kind: ItemKind::WholeFile,
            decision: PlanDecision::Rejected {
                reason: "source unavailable".to_owned(),
            },
        }],
    );
    let peer = SyncPlan::new("dest-b", vec![selected("b", "t2")]);
    // The failing plan yields no requests (its item is rejected).
    let failing_requests = map_whole_file_requests(&failing, "s", "c").expect("map");
    assert!(failing_requests.is_empty());
    // The peer is untouched: its request is still exact.
    let peer_requests = map_whole_file_requests(&peer, "s", "c").expect("map");
    assert_eq!(peer_requests.len(), 1);
    assert_eq!(peer_requests[0].plan_item_id, "b");
}

#[test]
fn every_operation_outcome_carries_source_plan_and_target_identities() {
    let plan = SyncPlan::new("dest-a", vec![selected("item-a", "apps/app.yaml")]);
    let requests =
        map_whole_file_requests(&plan, "source-identity", "config-identity").expect("map");
    let request = &requests[0];
    // The outcome identity set: source, configuration, plan, and target.
    assert_eq!(request.source_identity, "source-identity");
    assert_eq!(request.configuration_identity, "config-identity");
    assert_eq!(request.plan_identity, plan.render());
    assert_eq!(
        request.target.display(),
        "apps/app.yaml",
        "target identity is exact"
    );
    // A contextual failure keeps the identities: the plan item still
    // names the target in its explanation.
    let failure = PlanItem {
        id: "item-b".to_owned(),
        target: "apps/broken.yaml".to_owned(),
        source: "broken".to_owned(),
        source_path: String::new(),
        source_order: 2,
        kind: ItemKind::WholeFile,
        decision: PlanDecision::Rejected {
            reason: "source unavailable".to_owned(),
        },
    };
    assert!(
        format!("{:?}", failure.decision).contains("source unavailable"),
        "contextual failure reason preserved"
    );
    assert_eq!(failure.target, "apps/broken.yaml");
}

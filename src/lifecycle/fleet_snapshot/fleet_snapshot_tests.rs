//! Focused proof for the frozen repository snapshot of one admitted
//! destination.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::fleet_snapshot::build_frozen_snapshot;
use crate::lifecycle::sync_plan::PlanDecision;
use crate::lifecycle::sync_plan::{PlanItem, SyncPlan};
use crate::repository::{
    GitFacts, HeadState, IndexState, RepositorySnapshot, UpstreamState, WorktreeState,
};
use crate::source::ItemKind;
use std::{fs, path::Path, process::Command};

fn fixture_base() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("fleet-snapshot-")
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
    git(&["config", "user.name", "Snapshot"]);
    git(&["config", "user.email", "snapshot@example.test"]);
}

fn plan(repository: &str, items: Vec<PlanItem>) -> SyncPlan {
    SyncPlan::new(repository, items)
}

fn item(id: &str, target: &str, source: &str, order: usize) -> PlanItem {
    PlanItem {
        id: id.to_owned(),
        target: target.to_owned(),
        source: source.to_owned(),
        source_order: order,
        kind: ItemKind::WholeFile,
        decision: PlanDecision::Selected {
            reason: "inferred".to_owned(),
        },
    }
}

#[test]
fn an_existing_managed_file_gets_its_observed_identity() {
    let fixture = fixture_base();
    let root = fixture.path().join("destination");
    git_repo(&root);
    fs::write(root.join("managed.txt"), "v1\n").expect("file");
    git(&root, &["add", "."]);
    git(&root, &["commit", "--quiet", "--message", "base"]);
    let plan = plan("repo-a", vec![item("item-1", "managed.txt", "source-a", 0)]);
    let snapshot = build_frozen_snapshot(&root, &plan).expect("snapshot");
    assert_eq!(snapshot.targets().len(), 1);
    let target = &snapshot.targets()[0];
    let observed = target.observed_file().expect("observed identity");
    // The observed identity matches the real file's inode.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(root.join("managed.txt")).expect("meta");
        assert_eq!(observed.object().inode(), metadata.ino());
    }
    // The facts are captured from the real repository.
    let facts = snapshot.facts();
    let attached = matches!(
        facts.git(),
        crate::repository::GitRepositoryState::Git(git_facts)
            if matches!(git_facts.head(), HeadState::Attached { .. })
    );
    assert!(attached, "the snapshot freezes the attached head");
}

#[test]
fn a_missing_managed_file_is_an_absent_target() {
    let fixture = fixture_base();
    let root = fixture.path().join("destination");
    git_repo(&root);
    git(
        &root,
        &["commit", "--quiet", "--allow-empty", "--message", "base"],
    );
    let plan = plan("repo-a", vec![item("item-1", "absent.txt", "source-a", 0)]);
    let snapshot = build_frozen_snapshot(&root, &plan).expect("snapshot");
    let target = &snapshot.targets()[0];
    assert!(
        target.observed_file().is_none(),
        "an absent file is a lawful creation target"
    );
}

#[test]
fn the_witnesses_carry_the_frozen_base_head() {
    let fixture = fixture_base();
    let root = fixture.path().join("destination");
    git_repo(&root);
    fs::write(root.join("managed.txt"), "v1\n").expect("file");
    git(&root, &["add", "."]);
    git(&root, &["commit", "--quiet", "--message", "base"]);
    let head = git_text(&root, &["rev-parse", "HEAD"]);
    let plan = plan("repo-a", vec![item("item-1", "managed.txt", "source-a", 0)]);
    let snapshot = build_frozen_snapshot(&root, &plan).expect("snapshot");
    let base = snapshot
        .witnesses()
        .base_head()
        .map(|revision| revision.as_str().to_owned());
    assert_eq!(
        base,
        Some(head),
        "the frozen base-HEAD is the captured head"
    );
}

#[test]
fn a_non_regular_file_fails_typed() {
    let fixture = fixture_base();
    let root = fixture.path().join("destination");
    git_repo(&root);
    git(
        &root,
        &["commit", "--quiet", "--allow-empty", "--message", "base"],
    );
    #[cfg(unix)]
    {
        let fifo = root.join("managed-fifo");
        let status = Command::new("mkfifo").arg(&fifo).status().expect("mkfifo");
        assert!(status.success());
    }
    let plan = plan(
        "repo-a",
        vec![item("item-1", "managed-fifo", "source-a", 0)],
    );
    assert!(build_frozen_snapshot(&root, &plan).is_err());
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .current_dir(root)
        .output()
        .expect("git");
    assert!(output.status.success(), "git {args:?}: {:?}", output);
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

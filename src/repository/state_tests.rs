//! TDD contract for the pure repository-state domain.
//!
//! This target includes the product modules directly because the product is a
//! binary-only crate.  The domain must remain independent of Git, filesystem,
//! process, and network effects.

#![allow(clippy::too_many_lines, dead_code, unused_imports)]

use super::state::*;
/*
    AuthorityIdentity, AuthorizedChange, AuthorizedDelta, BaselineIdentityProof,
    CANONICAL_REPOSITORY_STATE_VERSION, CanonicalRepresentation, CausationAssessment,
    CausationBasis, CausationRelation, CheckWitness, DirectCausationProof, DirtyProvenance,
    EntryKind, FileIdentity, FilesystemClass, FilesystemIdentity, FrozenWitnesses, GitFacts,
    GitRepositoryState, HeadState, IndexEntry, IndexState, InferredCausation, ManagedOwnership,
    ManagedPathFailureProof, ManagedSectionId, ManagedTargetIdentity, ObjectIdentity, ObservedFact,
    OwnerDecision, RefName, RelativePath, RepositoryFacts, RepositoryId, RepositoryRoot,
    RepositorySnapshot, RevisionId, TargetChange, UpstreamState, WorktreeEntry, WorktreeState,
*/

fn repository_id(value: &str) -> RepositoryId {
    RepositoryId::new(value).expect("valid repository ID")
}

fn root(value: &str) -> RepositoryRoot {
    RepositoryRoot::new(value, authority(1)).expect("valid absolute repository root")
}

fn root_with_authority(value: &str, identity: AuthorityIdentity) -> RepositoryRoot {
    RepositoryRoot::new(value, identity).expect("valid absolute repository root")
}

fn path(value: &str) -> RelativePath {
    RelativePath::new(value).expect("valid relative path")
}

fn revision(value: &str) -> RevisionId {
    RevisionId::new(value).expect("valid revision")
}

fn ref_name(value: &str) -> RefName {
    RefName::new(value).expect("valid ref name")
}

fn witness(value: &str) -> CheckWitness {
    CheckWitness::new(value).expect("valid check witness")
}

fn identity(inode: u64) -> FileIdentity {
    FileIdentity::new(
        FilesystemIdentity::new(FilesystemClass::LinuxExtFamily, 7, 9),
        ObjectIdentity::new(7, inode),
        EntryKind::RegularFile,
        0o100644,
    )
    .expect("valid file identity")
}

fn authority(inode: u64) -> AuthorityIdentity {
    AuthorityIdentity::new(
        FilesystemIdentity::new(FilesystemClass::LinuxExtFamily, 7, 9),
        ObjectIdentity::new(7, inode),
    )
    .expect("valid authority identity")
}

fn witnesses() -> FrozenWitnesses {
    FrozenWitnesses::new(
        "authority-1",
        "source-1",
        "catalog-1",
        "configuration-1",
        "plan-1",
        vec![witness("check-a"), witness("check-b")],
        Some(revision("base-1")),
    )
    .expect("valid frozen witnesses")
}

fn facts() -> RepositoryFacts {
    RepositoryFacts::new(
        repository_id("destination-a"),
        root("/workspace/destination-a"),
        GitRepositoryState::Git(
            GitFacts::new(
                HeadState::Attached {
                    branch: ref_name("refs/heads/main"),
                    commit: revision("head-1"),
                },
                UpstreamState::Configured {
                    remote: "origin".into(),
                    reference: ref_name("refs/heads/main"),
                    commit: revision("remote-1"),
                },
                IndexState::Clean,
                WorktreeState::Clean,
            )
            .expect("valid Git facts"),
        ),
    )
    .expect("valid repository facts")
}

fn snapshot_with_target(path_value: &str) -> RepositorySnapshot {
    let target =
        ManagedTargetIdentity::whole_file(path(path_value), Some(identity(11))).expect("target");
    RepositorySnapshot::new(facts(), witnesses(), vec![target]).expect("snapshot")
}

#[test]
fn repository_identity_and_root_are_distinct_validated_values() {
    let repository = RepositoryId::new("repo-a").expect("ID");
    let repository_same = RepositoryId::new("repo-a").expect("ID");
    assert_eq!(repository, repository_same);
    assert_eq!(repository.as_str(), "repo-a");

    let repository_root = root("/srv/repositories/repo-a");
    assert_eq!(repository_root.as_str(), "/srv/repositories/repo-a");
    assert_ne!(repository.as_str(), repository_root.as_str());
}

#[test]
fn repository_paths_reject_escaping_and_empty_references() {
    for invalid in ["", "/absolute", "../escape", "a/../../escape", "a\0b"] {
        assert!(RelativePath::new(invalid).is_err(), "accepted {invalid:?}");
    }

    assert_eq!(path("a//./b").as_bytes(), b"a/b");
    assert_eq!(path("nested/file.txt").components().count(), 2);
}

#[test]
fn repository_root_rejects_non_absolute_parent_and_non_utf8_forms() {
    for invalid in ["", "relative/repo", "/tmp/../repo", "/tmp/./repo"] {
        assert!(
            RepositoryRoot::new(invalid, authority(1)).is_err(),
            "accepted {invalid:?}"
        );
    }

    let escaped = RepositoryRoot::new("/tmp/../repo", authority(1)).expect_err("parent traversal");
    assert_eq!(
        escaped.to_string(),
        "invalid absolute repository root \"/tmp/../repo\""
    );
}

#[test]
fn git_and_non_git_states_are_explicit() {
    let non_git = RepositoryFacts::new(
        repository_id("plain"),
        root("/workspace/plain"),
        GitRepositoryState::NonGit,
    )
    .expect("non-Git repository facts");
    assert!(matches!(non_git.git(), GitRepositoryState::NonGit));

    let git = facts();
    assert!(matches!(git.git(), GitRepositoryState::Git(_)));
}

#[test]
fn head_ref_and_upstream_variants_capture_unborn_detached_and_configured_states() {
    assert!(matches!(HeadState::Unborn, HeadState::Unborn));
    assert!(matches!(
        HeadState::Detached {
            commit: revision("detached"),
        },
        HeadState::Detached { .. }
    ));
    assert!(matches!(UpstreamState::Absent, UpstreamState::Absent));

    let git = GitFacts::new(
        HeadState::Detached {
            commit: revision("detached"),
        },
        UpstreamState::Absent,
        IndexState::Entries(vec![
            IndexEntry::new(
                path("staged.txt"),
                TargetChange::Added,
                DirtyProvenance::PreExisting,
            )
            .expect("index entry"),
        ]),
        WorktreeState::Entries(vec![
            WorktreeEntry::new(
                path("untracked.txt"),
                TargetChange::Untracked,
                DirtyProvenance::PreExisting,
            )
            .expect("worktree entry"),
        ]),
    )
    .expect("valid detached Git facts");
    assert!(matches!(git.head(), HeadState::Detached { .. }));
    assert!(matches!(git.upstream(), UpstreamState::Absent));
    assert_eq!(git.index().entries().len(), 1);
    assert_eq!(git.worktree().entries().len(), 1);
}

#[test]
fn index_and_worktree_entries_have_deterministic_order() {
    let git = GitFacts::new(
        HeadState::Unborn,
        UpstreamState::Absent,
        IndexState::Entries(vec![
            IndexEntry::new(
                path("z.txt"),
                TargetChange::Modified,
                DirtyProvenance::PreExisting,
            )
            .expect("index entry"),
            IndexEntry::new(
                path("a.txt"),
                TargetChange::Added,
                DirtyProvenance::PreExisting,
            )
            .expect("index entry"),
        ]),
        WorktreeState::Entries(vec![
            WorktreeEntry::new(
                path("z.txt"),
                TargetChange::Modified,
                DirtyProvenance::PreExisting,
            )
            .expect("worktree entry"),
            WorktreeEntry::new(
                path("a.txt"),
                TargetChange::Deleted,
                DirtyProvenance::PreExisting,
            )
            .expect("worktree entry"),
        ]),
    )
    .expect("valid dirty Git facts");
    assert_eq!(git.index().entries()[0].path().as_bytes(), b"a.txt");
    assert_eq!(git.worktree().entries()[0].path().as_bytes(), b"a.txt");
}

#[test]
fn managed_target_identity_preserves_whole_file_and_partial_scope() {
    let whole = ManagedTargetIdentity::whole_file(path("AGENTS.md"), Some(identity(1)))
        .expect("whole target");
    let section = ManagedTargetIdentity::section(
        path("AGENTS.md"),
        ManagedSectionId::new("runtime").expect("section ID"),
        Some(identity(1)),
    )
    .expect("section target");

    assert!(matches!(whole.ownership(), ManagedOwnership::WholeFile));
    assert!(matches!(
        section.ownership(),
        ManagedOwnership::Section { .. }
    ));
    assert_ne!(whole, section);
}

#[test]
fn managed_section_ids_use_the_exact_lowercase_stable_grammar() {
    assert!(ManagedSectionId::new("runtime.v1").is_ok());
    for invalid in [
        "",
        "Runtime",
        "runtime section",
        "runtime/slash",
        "runtime\0id",
    ] {
        assert!(
            ManagedSectionId::new(invalid).is_err(),
            "accepted {invalid:?}"
        );
    }

    let invalid_case = ManagedSectionId::new("Runtime").expect_err("uppercase section ID");
    assert_eq!(
        invalid_case.to_string(),
        "invalid managed section ID \"Runtime\""
    );
}

#[test]
fn duplicate_managed_target_and_duplicate_check_witnesses_are_rejected() {
    let duplicate_target = ManagedTargetIdentity::whole_file(path("same"), Some(identity(1)))
        .and_then(|target| {
            RepositorySnapshot::new(facts(), witnesses(), vec![target.clone(), target])
        });
    assert!(duplicate_target.is_err());

    assert!(
        FrozenWitnesses::new(
            "authority-1",
            "source-1",
            "catalog-1",
            "configuration-1",
            "plan-1",
            vec![witness("same"), witness("same")],
            None,
        )
        .is_err()
    );
}

#[test]
fn snapshot_is_ordered_and_keeps_frozen_witnesses() {
    let first =
        ManagedTargetIdentity::whole_file(path("z.txt"), Some(identity(2))).expect("target");
    let second =
        ManagedTargetIdentity::whole_file(path("a.txt"), Some(identity(1))).expect("target");
    let snapshot =
        RepositorySnapshot::new(facts(), witnesses(), vec![first, second]).expect("snapshot");

    assert_eq!(snapshot.facts().repository_id().as_str(), "destination-a");
    assert_eq!(snapshot.targets()[0].path().as_bytes(), b"a.txt");
    assert_eq!(snapshot.targets()[1].path().as_bytes(), b"z.txt");
    assert_eq!(snapshot.witnesses().checks()[0].as_str(), "check-a");
    assert_eq!(snapshot.witnesses().base_head().unwrap().as_str(), "base-1");
}

#[test]
fn frozen_checks_preserve_declared_order_while_rejecting_duplicates() {
    let frozen = FrozenWitnesses::new(
        "authority-1",
        "source-1",
        "catalog-1",
        "configuration-1",
        "plan-1",
        vec![witness("check-b"), witness("check-a")],
        None,
    )
    .expect("valid declared check order");
    assert_eq!(frozen.checks()[0].as_str(), "check-b");
    assert_eq!(frozen.checks()[1].as_str(), "check-a");
}

#[test]
fn authorized_delta_is_scoped_to_exact_managed_items_and_sorted() {
    let section = ManagedTargetIdentity::section(
        path("settings.toml"),
        ManagedSectionId::new("shared").expect("section ID"),
        Some(identity(9)),
    )
    .expect("section target");
    let whole = ManagedTargetIdentity::whole_file(path("README.md"), None).expect("whole target");
    let snapshot =
        RepositorySnapshot::new(facts(), witnesses(), vec![whole.clone(), section.clone()])
            .expect("snapshot");
    let delta = AuthorizedDelta::from_snapshot(
        &snapshot,
        vec![
            AuthorizedChange::new(whole, TargetChange::Added, None, Some(identity(10)))
                .expect("whole change"),
            AuthorizedChange::new(
                section,
                TargetChange::Modified,
                Some(identity(9)),
                Some(identity(11)),
            )
            .expect("section change"),
        ],
    )
    .expect("authorized delta");

    assert_eq!(delta.changes()[0].target().path().as_bytes(), b"README.md");
    assert_eq!(
        delta.changes()[1].target().path().as_bytes(),
        b"settings.toml"
    );
    assert_eq!(delta.repository_id().as_str(), "destination-a");
}

#[test]
fn authorized_delta_rejects_invalid_before_after_shapes() {
    let target = ManagedTargetIdentity::whole_file(path("file"), None).expect("target");
    assert!(
        AuthorizedChange::new(target.clone(), TargetChange::Added, Some(identity(1)), None)
            .is_err()
    );
    assert!(AuthorizedChange::new(target, TargetChange::Deleted, None, Some(identity(1))).is_err());
}

#[test]
fn whole_file_and_section_scopes_cannot_overlap() {
    let whole = ManagedTargetIdentity::whole_file(path("settings.toml"), None).expect("whole");
    let section = ManagedTargetIdentity::section(
        path("settings.toml"),
        ManagedSectionId::new("shared").expect("section ID"),
        None,
    )
    .expect("section");
    assert!(
        RepositorySnapshot::new(facts(), witnesses(), vec![whole.clone(), section.clone()])
            .is_err()
    );
}

#[test]
fn untracked_worktree_fact_cannot_become_authorized_delta() {
    let target = ManagedTargetIdentity::whole_file(path("new.txt"), None).expect("target");
    assert!(
        AuthorizedChange::new(target, TargetChange::Untracked, None, Some(identity(6))).is_err()
    );
}

#[test]
fn observed_facts_owner_decisions_and_inferred_causation_are_distinct_types() {
    let fact = ObservedFact::new(facts());
    let decision = OwnerDecision::new("managed bytes only");
    let inference =
        InferredCausation::new(CausationRelation::Uncertain, CausationBasis::NotEstablished)
            .expect("uncertain causation");

    assert_eq!(fact.value().repository_id().as_str(), "destination-a");
    assert_eq!(*decision.value(), "managed bytes only");
    assert_eq!(inference.relation(), CausationRelation::Uncertain);
    assert_eq!(inference.basis(), CausationBasis::NotEstablished);
}

#[test]
fn causation_requires_evidence_for_direct_and_uncertain_is_not_direct() {
    assert!(
        InferredCausation::new(CausationRelation::Direct, CausationBasis::NotEstablished).is_err()
    );
    assert!(
        InferredCausation::new(
            CausationRelation::Unrelated,
            CausationBasis::FailureEvidence
        )
        .is_err()
    );

    let uncertain =
        CausationAssessment::new(CausationRelation::Uncertain, CausationBasis::NotEstablished)
            .expect("uncertain causation");
    assert!(!uncertain.is_repair_eligible());
}

#[test]
fn file_identity_keeps_filesystem_object_kind_and_mode_together() {
    let value = identity(42);
    assert_eq!(value.filesystem().device(), 7);
    assert_eq!(value.object().inode(), 42);
    assert_eq!(value.kind(), EntryKind::RegularFile);
    assert_eq!(value.mode(), 0o100644);
}

#[test]
fn unsupported_filesystem_class_remains_a_fact_not_an_inferred_policy() {
    let identity = FilesystemIdentity::new(FilesystemClass::other("network").expect("class"), 1, 2);
    assert_eq!(
        identity.class(),
        &FilesystemClass::other("network").expect("class")
    );
}

#[test]
fn authority_identity_requires_filesystem_and_root_object_device_match() {
    let identity = authority(17);
    assert_eq!(identity.filesystem().device(), 7);
    assert_eq!(identity.object().inode(), 17);
    assert_eq!(
        root_with_authority("/workspace/destination-a", identity.clone()).authority(),
        &identity
    );

    let mismatch = AuthorityIdentity::new(
        FilesystemIdentity::new(FilesystemClass::LinuxExtFamily, 7, 9),
        ObjectIdentity::new(8, 17),
    )
    .expect_err("authority device mismatch");
    assert_eq!(
        mismatch.to_string(),
        "authority filesystem/object device mismatch: filesystem=7, object=8"
    );
}

#[test]
fn repository_root_cannot_be_created_without_a_canonical_authority_identity() {
    assert!(RepositoryRoot::new("/workspace/destination-a", authority(1)).is_ok());
    assert!(RepositoryRoot::new("/workspace/destination-a", authority(2)).is_ok());
    assert_ne!(
        RepositoryRoot::new("/workspace/destination-a", authority(1)).unwrap(),
        RepositoryRoot::new("/workspace/destination-a", authority(2)).unwrap()
    );
}

#[test]
fn snapshot_and_delta_share_the_exact_frozen_identity_and_target_set() {
    let snapshot = snapshot_with_target("managed.txt");
    let target = snapshot.targets()[0].clone();
    let delta = AuthorizedDelta::from_snapshot(
        &snapshot,
        vec![
            AuthorizedChange::new(
                target.clone(),
                TargetChange::Modified,
                Some(identity(11)),
                Some(identity(12)),
            )
            .expect("authorized target change"),
        ],
    )
    .expect("delta bound to snapshot");

    assert_eq!(
        delta.snapshot_identity(),
        &snapshot.canonical_representation()
    );
    assert_eq!(delta.authority_identity(), &snapshot.identity());
    assert_eq!(delta.repository_id(), snapshot.facts().repository_id());
    assert_eq!(delta.witnesses(), snapshot.witnesses());
    assert_eq!(delta.frozen_targets(), snapshot.targets());
    assert_eq!(delta.base_head(), snapshot.witnesses().base_head());
}

#[test]
fn delta_rejects_changes_outside_the_frozen_snapshot_targets() {
    let snapshot = snapshot_with_target("managed.txt");
    let outside = ManagedTargetIdentity::whole_file(path("outside.txt"), None).expect("outside");
    let error = AuthorizedDelta::from_snapshot(
        &snapshot,
        vec![
            AuthorizedChange::new(outside, TargetChange::Added, None, Some(identity(13)))
                .expect("shape-valid but unauthorized change"),
        ],
    )
    .expect_err("delta must reject targets outside the frozen snapshot");
    assert!(error.to_string().contains("frozen snapshot"));
}

#[test]
fn delta_rejects_renames_when_the_source_is_outside_frozen_targets() {
    let target =
        ManagedTargetIdentity::whole_file(path("new.txt"), Some(identity(11))).expect("target");
    let snapshot =
        RepositorySnapshot::new(facts(), witnesses(), vec![target.clone()]).expect("snapshot");
    let rename = AuthorizedChange::renamed(
        path("outside.txt"),
        target,
        Some(identity(11)),
        Some(identity(12)),
    )
    .expect("shape-valid rename");

    assert!(AuthorizedDelta::from_snapshot(&snapshot, vec![rename]).is_err());
}

#[test]
fn delta_accepts_a_rename_when_both_paths_are_frozen_with_matching_scope() {
    let old_target =
        ManagedTargetIdentity::whole_file(path("old.txt"), Some(identity(11))).expect("old");
    let new_target =
        ManagedTargetIdentity::whole_file(path("new.txt"), Some(identity(11))).expect("new");
    let snapshot =
        RepositorySnapshot::new(facts(), witnesses(), vec![old_target, new_target.clone()])
            .expect("snapshot");
    let rename = AuthorizedChange::renamed(
        path("old.txt"),
        new_target,
        Some(identity(11)),
        Some(identity(12)),
    )
    .expect("rename");

    let delta = AuthorizedDelta::from_snapshot(&snapshot, vec![rename]).expect("delta");
    assert_eq!(
        delta.changes()[0].rename_from().unwrap().as_bytes(),
        b"old.txt"
    );
}

#[test]
fn rename_changes_keep_both_old_and_new_paths() {
    let new_target =
        ManagedTargetIdentity::whole_file(path("new.txt"), Some(identity(14))).expect("new target");
    let rename = AuthorizedChange::renamed(
        path("old.txt"),
        new_target.clone(),
        Some(identity(14)),
        Some(identity(15)),
    )
    .expect("rename");
    assert_eq!(rename.rename_from().unwrap().as_bytes(), b"old.txt");
    assert_eq!(rename.target().path().as_bytes(), b"new.txt");
    assert!(
        AuthorizedChange::new(
            new_target,
            TargetChange::Renamed,
            Some(identity(14)),
            Some(identity(15)),
        )
        .is_err()
    );
}

#[test]
fn dirty_entries_are_provenanced_and_empty_entry_states_are_invalid() {
    let index = IndexEntry::new(
        path("staged.txt"),
        TargetChange::Modified,
        DirtyProvenance::CurrentOperation,
    )
    .expect("indexed entry");
    let worktree = WorktreeEntry::new(
        path("local.txt"),
        TargetChange::Modified,
        DirtyProvenance::PreExisting,
    )
    .expect("worktree entry");
    assert_eq!(index.provenance(), DirtyProvenance::CurrentOperation);
    assert_eq!(worktree.provenance(), DirtyProvenance::PreExisting);

    assert!(
        GitFacts::new(
            HeadState::Unborn,
            UpstreamState::Absent,
            IndexState::Entries(vec![]),
            WorktreeState::Entries(vec![]),
        )
        .is_err()
    );
}

#[test]
fn direct_causation_requires_a_matching_baseline_or_managed_failure_proof() {
    let snapshot = snapshot_with_target("managed.txt");
    let baseline = BaselineIdentityProof::from_snapshot(&snapshot, &snapshot)
        .expect("matching baseline proof");
    let direct = InferredCausation::direct(DirectCausationProof::Baseline(baseline));
    assert!(direct.is_repair_eligible());
    assert!(direct.proof().is_some());

    let delta = AuthorizedDelta::from_snapshot(
        &snapshot,
        vec![
            AuthorizedChange::new(
                snapshot.targets()[0].clone(),
                TargetChange::Modified,
                Some(identity(11)),
                Some(identity(12)),
            )
            .expect("change"),
        ],
    )
    .expect("delta");
    let failure = ManagedPathFailureProof::new(
        &snapshot,
        &delta,
        snapshot.targets()[0].clone(),
        "managed-content-application-failed",
    )
    .expect("managed failure proof");
    let direct_failure = InferredCausation::direct(DirectCausationProof::ManagedPath(failure));
    assert!(direct_failure.is_repair_eligible());
    assert_eq!(direct_failure.basis(), CausationBasis::FailureEvidence);

    let uncertain = CausationAssessment::uncertain();
    assert!(!uncertain.is_repair_eligible());
    assert!(InferredCausation::try_direct_without_proof().is_err());
}

#[test]
fn mismatched_baseline_identity_is_not_direct_causation_evidence() {
    let baseline = snapshot_with_target("managed.txt");
    let other_facts = RepositoryFacts::new(
        repository_id("destination-a"),
        RepositoryRoot::new("/workspace/destination-a", authority(2)).expect("other root"),
        GitRepositoryState::NonGit,
    )
    .expect("other facts");
    let other = RepositorySnapshot::new(other_facts, witnesses(), vec![]).expect("other snapshot");
    assert!(BaselineIdentityProof::from_snapshot(&baseline, &other).is_err());
}

#[test]
fn baseline_proof_requires_the_full_versioned_snapshot_identity() {
    let expected = snapshot_with_target("managed.txt");
    let changed_target =
        ManagedTargetIdentity::whole_file(path("other.txt"), Some(identity(11))).expect("target");
    let observed = RepositorySnapshot::new(facts(), witnesses(), vec![changed_target])
        .expect("observed snapshot");

    assert!(BaselineIdentityProof::from_snapshot(&expected, &observed).is_err());
}

#[test]
fn managed_failure_proof_is_bound_to_exact_snapshot_operation_and_target() {
    let snapshot = snapshot_with_target("managed.txt");
    let target = snapshot.targets()[0].clone();
    let delta = AuthorizedDelta::from_snapshot(
        &snapshot,
        vec![
            AuthorizedChange::new(
                target.clone(),
                TargetChange::Modified,
                Some(identity(11)),
                Some(identity(12)),
            )
            .expect("change"),
        ],
    )
    .expect("delta");

    let proof = ManagedPathFailureProof::new(
        &snapshot,
        &delta,
        target.clone(),
        "managed-content-application-failed",
    )
    .expect("bound failure proof");
    assert_eq!(proof.target(), &target);
    assert_eq!(
        proof.snapshot_identity(),
        &snapshot.canonical_representation()
    );
    assert_eq!(proof.operation(), &delta.canonical_representation());

    let forged_target =
        ManagedTargetIdentity::whole_file(path("forged.txt"), None).expect("forged target");
    assert!(
        ManagedPathFailureProof::new(
            &snapshot,
            &delta,
            forged_target,
            "managed-content-application-failed",
        )
        .is_err()
    );

    let other_snapshot = RepositorySnapshot::new(facts(), witnesses(), vec![]).expect("other");
    assert!(
        ManagedPathFailureProof::new(
            &other_snapshot,
            &delta,
            target,
            "managed-content-application-failed",
        )
        .is_err()
    );
}

#[test]
fn rename_source_cannot_overlap_another_change_target_or_source() {
    let old_target =
        ManagedTargetIdentity::whole_file(path("old.txt"), Some(identity(11))).expect("old");
    let new_target =
        ManagedTargetIdentity::whole_file(path("new.txt"), Some(identity(11))).expect("new");
    let other_target =
        ManagedTargetIdentity::whole_file(path("other.txt"), Some(identity(11))).expect("other");
    let snapshot = RepositorySnapshot::new(
        facts(),
        witnesses(),
        vec![old_target, new_target.clone(), other_target],
    )
    .expect("snapshot");
    let first = AuthorizedChange::renamed(
        path("old.txt"),
        new_target,
        Some(identity(11)),
        Some(identity(12)),
    )
    .expect("first rename");
    let second_target = snapshot
        .targets()
        .iter()
        .find(|target| target.path().as_bytes() == b"old.txt")
        .expect("old target")
        .clone();
    let second = AuthorizedChange::renamed(
        path("other.txt"),
        second_target,
        Some(identity(11)),
        Some(identity(13)),
    )
    .expect("second rename");

    assert!(AuthorizedDelta::from_snapshot(&snapshot, vec![first, second]).is_err());
}

#[test]
fn canonical_representation_is_versioned_deterministic_and_byte_comparable() {
    let snapshot = snapshot_with_target("managed.txt");
    let first = snapshot.canonical_representation();
    let second = snapshot.canonical_representation();
    assert_eq!(first.version(), CANONICAL_REPOSITORY_STATE_VERSION);
    assert_eq!(first, second);
    assert!(!first.as_bytes().is_empty());
    assert_eq!(first.compare(&second), std::cmp::Ordering::Equal);

    let delta = AuthorizedDelta::from_snapshot(
        &snapshot,
        vec![
            AuthorizedChange::new(
                snapshot.targets()[0].clone(),
                TargetChange::Modified,
                Some(identity(11)),
                Some(identity(12)),
            )
            .expect("change"),
        ],
    )
    .expect("delta");
    let delta_form: CanonicalRepresentation = delta.canonical_representation();
    assert_eq!(delta_form.version(), CANONICAL_REPOSITORY_STATE_VERSION);
    assert_ne!(first, delta_form);
}

#[test]
fn canonical_representation_uses_explicit_versioned_tags_not_debug_text() {
    let representation = snapshot_with_target("managed.txt").canonical_representation();
    assert_eq!(representation.version(), CANONICAL_REPOSITORY_STATE_VERSION);
    assert!(representation.as_bytes().starts_with(b"OMNI"));
    assert!(
        !representation
            .as_bytes()
            .windows(b"RepositoryFacts".len())
            .any(|window| window == b"RepositoryFacts")
    );
}

#[test]
fn canonical_snapshot_and_delta_bytes_are_stable_after_input_reordering() {
    let first_target =
        ManagedTargetIdentity::whole_file(path("z.txt"), Some(identity(11))).expect("z");
    let second_target =
        ManagedTargetIdentity::whole_file(path("a.txt"), Some(identity(12))).expect("a");
    let first = RepositorySnapshot::new(
        facts(),
        witnesses(),
        vec![first_target.clone(), second_target.clone()],
    )
    .expect("first snapshot");
    let second = RepositorySnapshot::new(facts(), witnesses(), vec![second_target, first_target])
        .expect("second snapshot");
    assert_eq!(
        first.canonical_representation(),
        second.canonical_representation()
    );

    let first_delta = AuthorizedDelta::from_snapshot(
        &first,
        vec![
            AuthorizedChange::new(
                first.targets()[0].clone(),
                TargetChange::Modified,
                first.targets()[0].observed_file().cloned(),
                Some(identity(13)),
            )
            .expect("first change"),
            AuthorizedChange::new(
                first.targets()[1].clone(),
                TargetChange::Modified,
                first.targets()[1].observed_file().cloned(),
                Some(identity(14)),
            )
            .expect("second change"),
        ],
    )
    .expect("first delta");
    let second_delta = AuthorizedDelta::from_snapshot(
        &second,
        vec![
            AuthorizedChange::new(
                second.targets()[0].clone(),
                TargetChange::Modified,
                second.targets()[0].observed_file().cloned(),
                Some(identity(13)),
            )
            .expect("second change"),
            AuthorizedChange::new(
                second.targets()[1].clone(),
                TargetChange::Modified,
                second.targets()[1].observed_file().cloned(),
                Some(identity(14)),
            )
            .expect("first change"),
        ],
    )
    .expect("second delta");
    assert_eq!(
        first_delta.canonical_representation(),
        second_delta.canonical_representation()
    );
}

#[test]
fn non_utf8_relative_paths_remain_exact_and_canonical() {
    let raw = b"configs/\xff.toml";
    let relative = RelativePath::from_bytes(raw);
    assert!(relative.is_ok());
    let relative = relative.unwrap();
    assert_eq!(relative.as_bytes(), raw.as_slice());

    let target = ManagedTargetIdentity::whole_file(relative.clone(), None).expect("target");
    let snapshot = RepositorySnapshot::new(facts(), witnesses(), vec![target]).expect("snapshot");
    assert!(
        snapshot
            .canonical_representation()
            .as_bytes()
            .windows(2)
            .any(|pair| pair == [0xFF, b'.'])
    );
}

#[test]
fn file_identity_rejects_filesystem_and_object_device_mismatch() {
    assert!(
        FileIdentity::new(
            FilesystemIdentity::new(FilesystemClass::LinuxExtFamily, 7, 9),
            ObjectIdentity::new(8, 42),
            EntryKind::RegularFile,
            0o100644,
        )
        .is_err()
    );
}

#[test]
fn unsupported_filesystem_class_rejects_empty_and_control_labels() {
    assert!(FilesystemClass::other("").is_err());
    assert!(FilesystemClass::other("network\0fs").is_err());
}

#[test]
fn authorized_change_requires_the_target_observed_identity_as_its_baseline() {
    let target =
        ManagedTargetIdentity::whole_file(path("managed.txt"), Some(identity(11))).expect("target");
    assert!(
        AuthorizedChange::new(
            target,
            TargetChange::Modified,
            Some(identity(12)),
            Some(identity(13)),
        )
        .is_err()
    );
}

#[test]
fn case_10_1_02_path_rename_scope_and_entry_boundaries_are_explicit() {
    let empty_components = RelativePath::from_bytes(b"./")
        .expect_err("a path containing only dot components is not a path");
    assert!(matches!(
        empty_components,
        DomainError::InvalidRelativePath { value } if value == "./"
    ));

    let rename_error =
        RenamePaths::new(path("same.txt"), path("same.txt")).expect_err("same rename paths");
    assert!(matches!(
        &rename_error,
        DomainError::InvalidRenamePaths { from, to }
            if from == "same.txt" && to == "same.txt"
    ));
    assert_eq!(
        rename_error.to_string(),
        "rename source and destination must differ: \"same.txt\" -> \"same.txt\""
    );

    let shared = ManagedSectionId::new("shared").expect("section ID");
    let same_path = path("settings.toml");
    let left = ManagedTargetIdentity::section(same_path.clone(), shared.clone(), None)
        .expect("left section");
    let right = ManagedTargetIdentity::section(same_path.clone(), shared, Some(identity(2)))
        .expect("right section");
    let error = RepositorySnapshot::new(facts(), witnesses(), vec![left, right])
        .expect_err("same named section scopes must conflict");
    assert!(matches!(
        &error,
        DomainError::ConflictingTarget { path } if path == "settings.toml"
    ));
    assert_eq!(
        error.to_string(),
        "conflicting managed target scope at \"settings.toml\""
    );

    let invalid_index = IndexEntry::new(
        path("staged.txt"),
        TargetChange::Untracked,
        DirtyProvenance::PreExisting,
    )
    .expect_err("index Untracked shape");
    assert!(matches!(
        &invalid_index,
        DomainError::InvalidChangeShape {
            change: TargetChange::Untracked
        }
    ));
    assert_eq!(
        invalid_index.to_string(),
        "invalid before/after shape for Untracked"
    );
    assert!(matches!(
        IndexEntry::new(
            path("staged.txt"),
            TargetChange::Renamed,
            DirtyProvenance::PreExisting
        ),
        Err(DomainError::InvalidChangeShape {
            change: TargetChange::Renamed
        })
    ));

    assert!(matches!(
        WorktreeEntry::new(
            path("local.txt"),
            TargetChange::Renamed,
            DirtyProvenance::PreExisting
        ),
        Err(DomainError::InvalidChangeShape {
            change: TargetChange::Renamed
        })
    ));
}

#[test]
fn case_10_1_03_dirty_state_validation_rejects_empty_duplicate_and_invalid_remote_values() {
    let duplicate_index_entry_a = IndexEntry::new(
        path("duplicate.txt"),
        TargetChange::Added,
        DirtyProvenance::PreExisting,
    )
    .expect("first index entry");
    let duplicate_index_entry_b = IndexEntry::new(
        path("duplicate.txt"),
        TargetChange::Modified,
        DirtyProvenance::CurrentOperation,
    )
    .expect("second index entry");
    let index_error = GitFacts::new(
        HeadState::Unborn,
        UpstreamState::Absent,
        IndexState::Entries(vec![duplicate_index_entry_a, duplicate_index_entry_b]),
        WorktreeState::Clean,
    )
    .expect_err("duplicate index paths must fail closed");
    assert!(matches!(
        index_error,
        DomainError::DuplicateValue { field: "index path", value } if value == "duplicate.txt"
    ));

    let duplicate_worktree_entry_a = WorktreeEntry::new(
        path("local.txt"),
        TargetChange::Deleted,
        DirtyProvenance::PreExisting,
    )
    .expect("first worktree entry");
    let duplicate_worktree_entry_b = WorktreeEntry::new(
        path("local.txt"),
        TargetChange::Untracked,
        DirtyProvenance::CurrentOperation,
    )
    .expect("second worktree entry");
    let worktree_error = GitFacts::new(
        HeadState::Unborn,
        UpstreamState::Absent,
        IndexState::Clean,
        WorktreeState::Entries(vec![duplicate_worktree_entry_a, duplicate_worktree_entry_b]),
    )
    .expect_err("duplicate worktree paths must fail closed");
    assert!(matches!(
        &worktree_error,
        DomainError::DuplicateValue { field: "worktree path", value } if value == "local.txt"
    ));
    assert_eq!(
        worktree_error.to_string(),
        "duplicate worktree path value \"local.txt\""
    );

    let empty_index = GitFacts::new(
        HeadState::Unborn,
        UpstreamState::Absent,
        IndexState::Entries(vec![]),
        WorktreeState::Clean,
    )
    .expect_err("empty index entries must fail closed");
    assert!(matches!(
        &empty_index,
        DomainError::EmptyEntries { field: "index" }
    ));
    assert_eq!(empty_index.to_string(), "index entries must not be empty");
    let empty_worktree = GitFacts::new(
        HeadState::Unborn,
        UpstreamState::Absent,
        IndexState::Clean,
        WorktreeState::Entries(vec![]),
    )
    .expect_err("empty worktree entries must fail closed");
    assert!(matches!(
        &empty_worktree,
        DomainError::EmptyEntries { field: "worktree" }
    ));
    assert_eq!(
        empty_worktree.to_string(),
        "worktree entries must not be empty"
    );

    let invalid_remote = GitFacts::new(
        HeadState::Unborn,
        UpstreamState::Configured {
            remote: String::new(),
            reference: ref_name("refs/heads/main"),
            commit: revision("remote"),
        },
        IndexState::Clean,
        WorktreeState::Clean,
    )
    .expect_err("empty upstream remote must fail closed");
    assert!(matches!(
        &invalid_remote,
        DomainError::EmptyValue {
            field: "upstream remote"
        }
    ));
    assert_eq!(
        invalid_remote.to_string(),
        "upstream remote must not be empty"
    );
    let control_remote = GitFacts::new(
        HeadState::Unborn,
        UpstreamState::Configured {
            remote: "origin\n".to_owned(),
            reference: ref_name("refs/heads/main"),
            commit: revision("remote"),
        },
        IndexState::Clean,
        WorktreeState::Clean,
    )
    .expect_err("control characters in upstream remote must fail closed");
    assert!(matches!(
        &control_remote,
        DomainError::ControlCharacter {
            field: "upstream remote"
        }
    ));
    assert_eq!(
        control_remote.to_string(),
        "upstream remote must not contain control characters"
    );
}

#[test]
fn case_10_1_04_rename_constructor_and_duplicate_delta_cases_are_typed() {
    let observed = identity(31);
    let target = ManagedTargetIdentity::whole_file(path("renamed-new.txt"), Some(observed.clone()))
        .expect("rename destination");
    let missing_before = AuthorizedChange::renamed(
        path("renamed-old.txt"),
        target.clone(),
        None,
        Some(identity(32)),
    )
    .expect_err("rename needs a before identity");
    assert!(matches!(
        missing_before,
        DomainError::InvalidChangeShape {
            change: TargetChange::Renamed
        }
    ));
    let mismatched_before = AuthorizedChange::renamed(
        path("renamed-old.txt"),
        target.clone(),
        Some(identity(30)),
        Some(identity(32)),
    )
    .expect_err("rename baseline must match the target observation");
    assert!(matches!(
        mismatched_before,
        DomainError::InvalidChangeShape {
            change: TargetChange::Renamed
        }
    ));

    let duplicate_target = ManagedTargetIdentity::whole_file(path("duplicate-target.txt"), None)
        .expect("duplicate delta target");
    let snapshot = RepositorySnapshot::new(facts(), witnesses(), vec![duplicate_target.clone()])
        .expect("snapshot");
    let first = AuthorizedChange::new(
        duplicate_target.clone(),
        TargetChange::Added,
        None,
        Some(identity(33)),
    )
    .expect("first duplicate-shaped change");
    let second = AuthorizedChange::new(
        duplicate_target,
        TargetChange::Added,
        None,
        Some(identity(34)),
    )
    .expect("second duplicate-shaped change");
    let error = AuthorizedDelta::from_snapshot(&snapshot, vec![first, second])
        .expect_err("duplicate authorized targets must fail closed");
    assert!(matches!(
        error,
        DomainError::DuplicateValue {
            field: "authorized target",
            value
        } if value == "duplicate-target.txt"
    ));
}

#[test]
fn case_10_1_05_causation_proofs_expose_frozen_values_and_exact_failure_text() {
    let snapshot = snapshot_with_target("causation.txt");
    let baseline = BaselineIdentityProof::from_snapshot(&snapshot, &snapshot)
        .expect("identical snapshots prove the baseline");

    let direct = InferredCausation::direct(DirectCausationProof::Baseline(baseline.clone()));
    assert_eq!(direct.relation(), CausationRelation::Direct);
    assert_eq!(direct.basis(), CausationBasis::BaselineComparison);
    assert!(direct.is_repair_eligible());
    let invalid_direct = InferredCausation::try_direct_without_proof()
        .expect_err("direct causation without proof must fail");
    assert_eq!(
        invalid_direct.to_string(),
        "invalid causation relation Direct with basis NotEstablished"
    );

    let target = snapshot.targets()[0].clone();
    let delta = AuthorizedDelta::from_snapshot(
        &snapshot,
        vec![
            AuthorizedChange::new(
                target.clone(),
                TargetChange::Modified,
                Some(identity(11)),
                Some(identity(12)),
            )
            .expect("change"),
        ],
    )
    .expect("delta");
    assert!(matches!(
        ManagedPathFailureProof::new(&snapshot, &delta, target, ""),
        Err(DomainError::EmptyValue {
            field: "managed path failure"
        })
    ));
}

#[test]
fn case_10_1_06_canonical_encoding_covers_filesystem_kind_state_and_change_tags() {
    fn matrix_file(
        class: FilesystemClass,
        device: u64,
        inode: u64,
        kind: EntryKind,
        mode: u32,
    ) -> FileIdentity {
        FileIdentity::new(
            FilesystemIdentity::new(class, device, 71),
            ObjectIdentity::new(device, inode),
            kind,
            mode,
        )
        .expect("matrix file identity")
    }

    let authority = AuthorityIdentity::new(
        FilesystemIdentity::new(FilesystemClass::MacOsApfs, 70, 71),
        ObjectIdentity::new(70, 700),
    )
    .expect("macOS authority identity");
    let directory = matrix_file(
        FilesystemClass::MacOsApfs,
        70,
        701,
        EntryKind::Directory,
        0o040755,
    );
    let directory_after = matrix_file(
        FilesystemClass::MacOsApfs,
        70,
        702,
        EntryKind::Directory,
        0o040755,
    );
    let symlink = matrix_file(
        FilesystemClass::MacOsApfs,
        70,
        703,
        EntryKind::Symlink,
        0o120777,
    );
    let other_class = FilesystemClass::other("network").expect("other filesystem class");
    let other = matrix_file(other_class.clone(), 70, 704, EntryKind::Other, 0);
    let other_after = matrix_file(other_class, 70, 705, EntryKind::Other, 0);
    let regular = matrix_file(
        FilesystemClass::LinuxExtFamily,
        70,
        706,
        EntryKind::RegularFile,
        0o100644,
    );

    let index_entries = vec![
        IndexEntry::new(
            path("index-added"),
            TargetChange::Added,
            DirtyProvenance::PreExisting,
        )
        .expect("index Added"),
        IndexEntry::new(
            path("index-deleted"),
            TargetChange::Deleted,
            DirtyProvenance::CurrentOperation,
        )
        .expect("index Deleted"),
        IndexEntry::new(
            path("index-modified"),
            TargetChange::Modified,
            DirtyProvenance::PreExisting,
        )
        .expect("index Modified"),
        IndexEntry::new(
            path("index-type"),
            TargetChange::TypeChanged,
            DirtyProvenance::CurrentOperation,
        )
        .expect("index TypeChanged"),
        IndexEntry::new(
            path("index-mode"),
            TargetChange::ModeChanged,
            DirtyProvenance::PreExisting,
        )
        .expect("index ModeChanged"),
        IndexEntry::new(
            path("index-link"),
            TargetChange::LinkChanged,
            DirtyProvenance::CurrentOperation,
        )
        .expect("index LinkChanged"),
        IndexEntry::renamed(
            path("index-old"),
            path("index-renamed"),
            DirtyProvenance::CurrentOperation,
        )
        .expect("index Renamed"),
    ];
    let worktree_entries = vec![
        WorktreeEntry::new(
            path("worktree-added"),
            TargetChange::Added,
            DirtyProvenance::PreExisting,
        )
        .expect("worktree Added"),
        WorktreeEntry::new(
            path("worktree-deleted"),
            TargetChange::Deleted,
            DirtyProvenance::CurrentOperation,
        )
        .expect("worktree Deleted"),
        WorktreeEntry::new(
            path("worktree-modified"),
            TargetChange::Modified,
            DirtyProvenance::PreExisting,
        )
        .expect("worktree Modified"),
        WorktreeEntry::new(
            path("worktree-type"),
            TargetChange::TypeChanged,
            DirtyProvenance::CurrentOperation,
        )
        .expect("worktree TypeChanged"),
        WorktreeEntry::new(
            path("worktree-mode"),
            TargetChange::ModeChanged,
            DirtyProvenance::PreExisting,
        )
        .expect("worktree ModeChanged"),
        WorktreeEntry::new(
            path("worktree-link"),
            TargetChange::LinkChanged,
            DirtyProvenance::CurrentOperation,
        )
        .expect("worktree LinkChanged"),
        WorktreeEntry::new(
            path("worktree-untracked"),
            TargetChange::Untracked,
            DirtyProvenance::PreExisting,
        )
        .expect("worktree Untracked"),
        WorktreeEntry::renamed(
            path("worktree-old"),
            path("worktree-renamed"),
            DirtyProvenance::CurrentOperation,
        )
        .expect("worktree Renamed"),
    ];
    let git = GitFacts::new(
        HeadState::Detached {
            commit: revision("detached-matrix"),
        },
        UpstreamState::Absent,
        IndexState::Entries(index_entries),
        WorktreeState::Entries(worktree_entries),
    )
    .expect("matrix Git facts");
    let facts = RepositoryFacts::new(
        repository_id("matrix"),
        root_with_authority("/workspace/matrix", authority),
        GitRepositoryState::Git(git),
    )
    .expect("matrix repository facts");

    let rename_old = ManagedTargetIdentity::whole_file(path("matrix-old"), Some(directory.clone()))
        .expect("rename source");
    let rename_new = ManagedTargetIdentity::whole_file(path("matrix-new"), Some(directory.clone()))
        .expect("rename destination");
    let symlink_target =
        ManagedTargetIdentity::whole_file(path("matrix-symlink"), Some(symlink.clone()))
            .expect("symlink target");
    let other_target = ManagedTargetIdentity::whole_file(path("matrix-other"), Some(other.clone()))
        .expect("other target");
    let regular_target =
        ManagedTargetIdentity::whole_file(path("matrix-regular"), Some(regular.clone()))
            .expect("regular target");
    let absent_target =
        ManagedTargetIdentity::whole_file(path("matrix-absent"), None).expect("absent target");
    let section_target = ManagedTargetIdentity::section(
        path("matrix-section"),
        ManagedSectionId::new("matrix").expect("section ID"),
        Some(other.clone()),
    )
    .expect("section target");
    let snapshot = RepositorySnapshot::new(
        facts,
        FrozenWitnesses::new(
            "authority-matrix",
            "source-matrix",
            "catalog-matrix",
            "configuration-matrix",
            "plan-matrix",
            vec![witness("check-matrix")],
            None,
        )
        .expect("witnesses without a base head"),
        vec![
            rename_old,
            rename_new.clone(),
            symlink_target.clone(),
            other_target.clone(),
            regular_target.clone(),
            absent_target.clone(),
            section_target.clone(),
        ],
    )
    .expect("matrix snapshot");

    let _snapshot_form = snapshot.canonical_representation();

    let changes = vec![
        AuthorizedChange::new(
            absent_target,
            TargetChange::Added,
            None,
            Some(regular.clone()),
        )
        .expect("Added change"),
        AuthorizedChange::new(
            symlink_target,
            TargetChange::Deleted,
            Some(symlink.clone()),
            None,
        )
        .expect("Deleted change"),
        AuthorizedChange::new(
            other_target.clone(),
            TargetChange::Modified,
            Some(other.clone()),
            Some(other_after),
        )
        .expect("Modified change"),
        AuthorizedChange::new(
            regular_target,
            TargetChange::TypeChanged,
            Some(regular.clone()),
            Some(directory.clone()),
        )
        .expect("TypeChanged change"),
        AuthorizedChange::new(
            section_target,
            TargetChange::ModeChanged,
            Some(other.clone()),
            Some(other.clone()),
        )
        .expect("ModeChanged change"),
        AuthorizedChange::renamed(
            path("matrix-old"),
            rename_new,
            Some(directory),
            Some(directory_after),
        )
        .expect("Renamed change"),
    ];
    let delta = AuthorizedDelta::from_snapshot(&snapshot, changes).expect("matrix delta");
    let delta_form = delta.canonical_representation();
    let mode_delta = AuthorizedDelta::from_snapshot(
        &snapshot,
        vec![
            AuthorizedChange::new(
                other_target,
                TargetChange::ModeChanged,
                Some(other.clone()),
                Some(other),
            )
            .expect("mode mutation change"),
        ],
    )
    .expect("mode mutation delta");
    assert_ne!(
        delta_form.as_bytes(),
        mode_delta.canonical_representation().as_bytes(),
        "authorized change tags must alter durable delta bytes"
    );

    let provenance_snapshot = |provenance| {
        RepositorySnapshot::new(
            RepositoryFacts::new(
                repository_id("provenance"),
                root("/workspace/provenance"),
                GitRepositoryState::Git(
                    GitFacts::new(
                        HeadState::Unborn,
                        UpstreamState::Absent,
                        IndexState::Clean,
                        WorktreeState::Entries(vec![
                            WorktreeEntry::new(
                                path("provenance.txt"),
                                TargetChange::Modified,
                                provenance,
                            )
                            .expect("provenance entry"),
                        ]),
                    )
                    .expect("provenance facts"),
                ),
            )
            .expect("provenance repository facts"),
            witnesses(),
            vec![],
        )
        .expect("provenance snapshot")
    };
    let pre_existing = provenance_snapshot(DirtyProvenance::PreExisting);
    let current_operation = provenance_snapshot(DirtyProvenance::CurrentOperation);
    assert_ne!(
        pre_existing.canonical_representation().as_bytes(),
        current_operation.canonical_representation().as_bytes(),
        "dirty provenance must alter durable snapshot bytes"
    );

    let unborn_facts = RepositoryFacts::new(
        repository_id("unborn"),
        root("/workspace/unborn"),
        GitRepositoryState::Git(
            GitFacts::new(
                HeadState::Unborn,
                UpstreamState::Absent,
                IndexState::Clean,
                WorktreeState::Clean,
            )
            .expect("unborn Git facts"),
        ),
    )
    .expect("unborn facts");
    let _unborn = RepositorySnapshot::new(unborn_facts, witnesses(), vec![])
        .expect("unborn snapshot")
        .canonical_representation();

    let _non_git = RepositorySnapshot::new(
        RepositoryFacts::new(
            repository_id("non-git"),
            root("/workspace/non-git"),
            GitRepositoryState::NonGit,
        )
        .expect("non-Git facts"),
        witnesses(),
        vec![],
    )
    .expect("non-Git snapshot")
    .canonical_representation();
}

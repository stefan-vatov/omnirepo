#![allow(clippy::too_many_lines, dead_code, unused_imports)]

use super::*;
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

pub(crate) fn repository_id(value: &str) -> RepositoryId {
    RepositoryId::new(value).expect("valid repository ID")
}

pub(crate) fn root(value: &str) -> RepositoryRoot {
    RepositoryRoot::new(value, authority(1)).expect("valid absolute repository root")
}

pub(crate) fn root_with_authority(value: &str, identity: AuthorityIdentity) -> RepositoryRoot {
    RepositoryRoot::new(value, identity).expect("valid absolute repository root")
}

pub(crate) fn path(value: &str) -> RelativePath {
    RelativePath::new(value).expect("valid relative path")
}

pub(crate) fn revision(value: &str) -> RevisionId {
    RevisionId::new(value).expect("valid revision")
}

pub(crate) fn ref_name(value: &str) -> RefName {
    RefName::new(value).expect("valid ref name")
}

pub(crate) fn witness(value: &str) -> CheckWitness {
    CheckWitness::new(value).expect("valid check witness")
}

pub(crate) fn identity(inode: u64) -> FileIdentity {
    FileIdentity::new(
        FilesystemIdentity::new(FilesystemClass::Linux, 7, 9),
        ObjectIdentity::new(7, inode),
        EntryKind::RegularFile,
        0o100644,
    )
    .expect("valid file identity")
}

pub(crate) fn authority(inode: u64) -> AuthorityIdentity {
    AuthorityIdentity::new(
        FilesystemIdentity::new(FilesystemClass::Linux, 7, 9),
        ObjectIdentity::new(7, inode),
    )
    .expect("valid authority identity")
}

pub(crate) fn witnesses() -> FrozenWitnesses {
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

pub(crate) fn facts() -> RepositoryFacts {
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

pub(crate) fn snapshot_with_target(path_value: &str) -> RepositorySnapshot {
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
        FilesystemIdentity::new(FilesystemClass::Linux, 7, 9),
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

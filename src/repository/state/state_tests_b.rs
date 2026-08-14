use super::state_tests_a::{
    facts, identity, path, ref_name, repository_id, revision, root, root_with_authority,
    snapshot_with_target, witness, witnesses,
};
use super::*;

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

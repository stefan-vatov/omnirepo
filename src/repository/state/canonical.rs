use super::{
    AuthorityIdentity, AuthorizedChange, AuthorizedDelta, DirtyProvenance, EntryKind, FileIdentity,
    FilesystemClass, FilesystemIdentity, FrozenWitnesses, GitFacts, GitRepositoryState, HeadState,
    IndexEntry, IndexState, ManagedOwnership, ManagedTargetIdentity, ObjectIdentity,
    RepositoryFacts, RepositoryRoot, RepositorySnapshot, TargetChange, UpstreamState,
    WorktreeEntry, WorktreeState,
};
pub const CANONICAL_REPOSITORY_STATE_VERSION: u16 = 2;

const CANONICAL_MAGIC: &[u8] = b"OMNI";
const DOCUMENT_SNAPSHOT: u8 = 1;
const DOCUMENT_DELTA: u8 = 2;

const RECORD_FACTS: u8 = 0x10;
const RECORD_ROOT: u8 = 0x11;
const RECORD_AUTHORITY: u8 = 0x12;
const RECORD_FILESYSTEM: u8 = 0x13;
const RECORD_FILESYSTEM_CLASS: u8 = 0x14;
const RECORD_OBJECT: u8 = 0x15;
const RECORD_FILE: u8 = 0x16;
const RECORD_TARGET: u8 = 0x17;
const RECORD_OWNERSHIP: u8 = 0x18;
const RECORD_INDEX_ENTRY: u8 = 0x19;
const RECORD_WORKTREE_ENTRY: u8 = 0x1a;
const RECORD_INDEX_STATE: u8 = 0x1b;
const RECORD_WORKTREE_STATE: u8 = 0x1c;
const RECORD_HEAD: u8 = 0x1d;
const RECORD_UPSTREAM: u8 = 0x1e;
const RECORD_GIT_FACTS: u8 = 0x1f;
const RECORD_GIT_STATE: u8 = 0x20;
const RECORD_WITNESSES: u8 = 0x21;
const RECORD_TARGETS: u8 = 0x22;
const RECORD_CHANGES: u8 = 0x23;
const RECORD_CHANGE: u8 = 0x24;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CanonicalRepresentation {
    version: u16,
    bytes: Vec<u8>,
}

impl CanonicalRepresentation {
    fn new(document: u8, fields: Vec<(u8, Vec<u8>)>) -> Self {
        let mut bytes = Vec::with_capacity(CANONICAL_MAGIC.len() + 3);
        bytes.extend_from_slice(CANONICAL_MAGIC);
        bytes.extend_from_slice(&CANONICAL_REPOSITORY_STATE_VERSION.to_be_bytes());
        bytes.push(document);
        append_fields(&mut bytes, &fields);
        Self {
            version: CANONICAL_REPOSITORY_STATE_VERSION,
            bytes,
        }
    }

    pub fn version(&self) -> u16 {
        self.version
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn compare(&self, other: &Self) -> std::cmp::Ordering {
        self.cmp(other)
    }
}

fn append_field(bytes: &mut Vec<u8>, tag: u8, value: &[u8]) {
    bytes.push(tag);
    bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
    bytes.extend_from_slice(value);
}

fn append_fields(bytes: &mut Vec<u8>, fields: &[(u8, Vec<u8>)]) {
    for (tag, value) in fields {
        append_field(bytes, *tag, value);
    }
}

fn record(tag: u8, fields: Vec<(u8, Vec<u8>)>) -> Vec<u8> {
    let mut bytes = vec![tag];
    append_fields(&mut bytes, &fields);
    bytes
}

fn sequence(tag: u8, values: Vec<Vec<u8>>) -> Vec<u8> {
    record(tag, values.into_iter().map(|value| (1, value)).collect())
}

fn optional(value: Option<Vec<u8>>) -> Vec<u8> {
    match value {
        None => vec![0],
        Some(value) => {
            let mut bytes = Vec::with_capacity(value.len() + 1);
            bytes.push(1);
            bytes.extend_from_slice(&value);
            bytes
        }
    }
}

fn u8_bytes(value: u8) -> Vec<u8> {
    vec![value]
}

fn u32_bytes(value: u32) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}

fn u64_bytes(value: u64) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}

fn text_bytes(value: &str) -> Vec<u8> {
    value.as_bytes().to_vec()
}

fn encode_filesystem_class(class: &FilesystemClass) -> Vec<u8> {
    match class {
        FilesystemClass::LinuxExtFamily => record(RECORD_FILESYSTEM_CLASS, vec![(1, vec![0])]),
        FilesystemClass::MacOsApfs => record(RECORD_FILESYSTEM_CLASS, vec![(1, vec![1])]),
        FilesystemClass::Other(name) => record(
            RECORD_FILESYSTEM_CLASS,
            vec![(1, vec![2]), (2, text_bytes(name.as_str()))],
        ),
    }
}

fn encode_filesystem(identity: &FilesystemIdentity) -> Vec<u8> {
    record(
        RECORD_FILESYSTEM,
        vec![
            (1, encode_filesystem_class(identity.class())),
            (2, u64_bytes(identity.device())),
            (3, u64_bytes(identity.mount_id())),
        ],
    )
}

fn encode_object(identity: ObjectIdentity) -> Vec<u8> {
    record(
        RECORD_OBJECT,
        vec![
            (1, u64_bytes(identity.device())),
            (2, u64_bytes(identity.inode())),
        ],
    )
}

fn encode_authority(identity: &AuthorityIdentity) -> Vec<u8> {
    record(
        RECORD_AUTHORITY,
        vec![
            (1, encode_filesystem(identity.filesystem())),
            (2, encode_object(identity.object())),
        ],
    )
}

fn encode_root(root: &RepositoryRoot) -> Vec<u8> {
    record(
        RECORD_ROOT,
        vec![
            (1, text_bytes(root.as_str())),
            (2, encode_authority(root.authority())),
        ],
    )
}

fn encode_file(identity: &FileIdentity) -> Vec<u8> {
    let kind = match identity.kind() {
        EntryKind::RegularFile => 0,
        EntryKind::Directory => 1,
        EntryKind::Symlink => 2,
        EntryKind::Other => 3,
    };
    record(
        RECORD_FILE,
        vec![
            (1, encode_filesystem(identity.filesystem())),
            (2, encode_object(*identity.object())),
            (3, u8_bytes(kind)),
            (4, u32_bytes(identity.mode())),
        ],
    )
}

fn encode_ownership(ownership: &ManagedOwnership) -> Vec<u8> {
    match ownership {
        ManagedOwnership::WholeFile => record(RECORD_OWNERSHIP, vec![(1, vec![0])]),
        ManagedOwnership::Section { id } => record(
            RECORD_OWNERSHIP,
            vec![(1, vec![1]), (2, text_bytes(id.as_str()))],
        ),
    }
}

fn encode_target(target: &ManagedTargetIdentity) -> Vec<u8> {
    record(
        RECORD_TARGET,
        vec![
            (1, target.path().as_bytes().to_vec()),
            (2, encode_ownership(target.ownership())),
            (3, optional(target.observed_file().map(encode_file))),
        ],
    )
}

fn encode_target_change(change: TargetChange) -> Vec<u8> {
    let value = match change {
        TargetChange::Added => 0,
        TargetChange::Deleted => 1,
        TargetChange::Modified => 2,
        TargetChange::Renamed => 3,
        TargetChange::TypeChanged => 4,
        TargetChange::ModeChanged => 5,
        TargetChange::LinkChanged => 6,
        TargetChange::Untracked => 7,
    };
    u8_bytes(value)
}

fn encode_provenance(provenance: DirtyProvenance) -> Vec<u8> {
    u8_bytes(match provenance {
        DirtyProvenance::PreExisting => 0,
        DirtyProvenance::CurrentOperation => 1,
    })
}

fn encode_index_entry(entry: &IndexEntry) -> Vec<u8> {
    record(
        RECORD_INDEX_ENTRY,
        vec![
            (1, entry.path().as_bytes().to_vec()),
            (2, encode_target_change(entry.change())),
            (3, encode_provenance(entry.provenance())),
            (
                4,
                optional(entry.rename_from().map(|path| path.as_bytes().to_vec())),
            ),
        ],
    )
}

fn encode_worktree_entry(entry: &WorktreeEntry) -> Vec<u8> {
    record(
        RECORD_WORKTREE_ENTRY,
        vec![
            (1, entry.path().as_bytes().to_vec()),
            (2, encode_target_change(entry.change())),
            (3, encode_provenance(entry.provenance())),
            (
                4,
                optional(entry.rename_from().map(|path| path.as_bytes().to_vec())),
            ),
        ],
    )
}

fn encode_index_state(state: &IndexState) -> Vec<u8> {
    match state {
        IndexState::Clean => record(RECORD_INDEX_STATE, vec![(1, vec![0])]),
        IndexState::Entries(entries) => record(
            RECORD_INDEX_STATE,
            vec![
                (1, vec![1]),
                (
                    2,
                    sequence(
                        RECORD_CHANGES,
                        entries.iter().map(encode_index_entry).collect(),
                    ),
                ),
            ],
        ),
    }
}

fn encode_worktree_state(state: &WorktreeState) -> Vec<u8> {
    match state {
        WorktreeState::Clean => record(RECORD_WORKTREE_STATE, vec![(1, vec![0])]),
        WorktreeState::Entries(entries) => record(
            RECORD_WORKTREE_STATE,
            vec![
                (1, vec![1]),
                (
                    2,
                    sequence(
                        RECORD_CHANGES,
                        entries.iter().map(encode_worktree_entry).collect(),
                    ),
                ),
            ],
        ),
    }
}

fn encode_head(head: &HeadState) -> Vec<u8> {
    match head {
        HeadState::Unborn => record(RECORD_HEAD, vec![(1, vec![0])]),
        HeadState::Detached { commit } => record(
            RECORD_HEAD,
            vec![(1, vec![1]), (3, text_bytes(commit.as_str()))],
        ),
        HeadState::Attached { branch, commit } => record(
            RECORD_HEAD,
            vec![
                (1, vec![2]),
                (2, text_bytes(branch.as_str())),
                (3, text_bytes(commit.as_str())),
            ],
        ),
    }
}

fn encode_upstream(upstream: &UpstreamState) -> Vec<u8> {
    match upstream {
        UpstreamState::Absent => record(RECORD_UPSTREAM, vec![(1, vec![0])]),
        UpstreamState::Configured {
            remote,
            reference,
            commit,
        } => record(
            RECORD_UPSTREAM,
            vec![
                (1, vec![1]),
                (2, text_bytes(remote)),
                (3, text_bytes(reference.as_str())),
                (4, text_bytes(commit.as_str())),
            ],
        ),
    }
}

fn encode_git_facts(facts: &GitFacts) -> Vec<u8> {
    record(
        RECORD_GIT_FACTS,
        vec![
            (1, encode_head(facts.head())),
            (2, encode_upstream(facts.upstream())),
            (3, encode_index_state(facts.index())),
            (4, encode_worktree_state(facts.worktree())),
        ],
    )
}

fn encode_git_state(state: &GitRepositoryState) -> Vec<u8> {
    match state {
        GitRepositoryState::NonGit => record(RECORD_GIT_STATE, vec![(1, vec![0])]),
        GitRepositoryState::Git(facts) => record(
            RECORD_GIT_STATE,
            vec![(1, vec![1]), (2, encode_git_facts(facts))],
        ),
    }
}

fn encode_repository_facts(facts: &RepositoryFacts) -> Vec<u8> {
    record(
        RECORD_FACTS,
        vec![
            (1, text_bytes(facts.repository_id().as_str())),
            (2, encode_root(facts.root())),
            (3, encode_git_state(facts.git())),
        ],
    )
}

fn encode_witnesses(witnesses: &FrozenWitnesses) -> Vec<u8> {
    record(
        RECORD_WITNESSES,
        vec![
            (1, text_bytes(witnesses.authority())),
            (2, text_bytes(witnesses.source())),
            (3, text_bytes(witnesses.catalog())),
            (4, text_bytes(witnesses.configuration())),
            (5, text_bytes(witnesses.plan())),
            (
                6,
                sequence(
                    RECORD_CHANGES,
                    witnesses
                        .checks()
                        .iter()
                        .map(|check| text_bytes(check.as_str()))
                        .collect(),
                ),
            ),
            (
                7,
                optional(
                    witnesses
                        .base_head()
                        .map(|revision| text_bytes(revision.as_str())),
                ),
            ),
        ],
    )
}

fn encode_authorized_change(change: &AuthorizedChange) -> Vec<u8> {
    record(
        RECORD_CHANGE,
        vec![
            (1, encode_target(change.target())),
            (2, encode_target_change(change.change())),
            (3, optional(change.before().map(encode_file))),
            (4, optional(change.after().map(encode_file))),
            (
                5,
                optional(change.rename_from().map(|path| path.as_bytes().to_vec())),
            ),
        ],
    )
}

impl RepositorySnapshot {
    pub fn canonical_representation(&self) -> CanonicalRepresentation {
        CanonicalRepresentation::new(
            DOCUMENT_SNAPSHOT,
            vec![
                (1, encode_repository_facts(&self.facts)),
                (2, encode_witnesses(&self.witnesses)),
                (
                    3,
                    sequence(
                        RECORD_TARGETS,
                        self.targets.iter().map(encode_target).collect(),
                    ),
                ),
            ],
        )
    }
}

impl AuthorizedDelta {
    pub fn canonical_representation(&self) -> CanonicalRepresentation {
        CanonicalRepresentation::new(
            DOCUMENT_DELTA,
            vec![
                (1, text_bytes(self.repository_id.as_str())),
                (2, encode_authority(&self.authority_identity)),
                (3, self.snapshot_identity.as_bytes().to_vec()),
                (4, encode_witnesses(&self.witnesses)),
                (
                    5,
                    sequence(
                        RECORD_TARGETS,
                        self.frozen_targets.iter().map(encode_target).collect(),
                    ),
                ),
                (
                    6,
                    optional(
                        self.base_head
                            .as_ref()
                            .map(|revision| text_bytes(revision.as_str())),
                    ),
                ),
                (
                    7,
                    sequence(
                        RECORD_CHANGES,
                        self.changes.iter().map(encode_authorized_change).collect(),
                    ),
                ),
            ],
        )
    }
}

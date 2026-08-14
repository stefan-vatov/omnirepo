use super::{
    AbsolutePath, AuthorityAdapterKind, AuthorityIdentity, MutationIntent, ObjectClass, PathError,
    RelativePath, backend,
};
use std::{collections::HashMap, marker::PhantomData, path::Path};

#[derive(Debug)]
pub struct MachineConfigRoot;
#[derive(Debug)]
pub struct SourceSnapshotRoot;
#[derive(Debug)]
pub struct DestinationRepositoryRoot;
#[derive(Debug)]
pub struct RunRecordRoot;
#[derive(Debug)]
pub struct ProcessWorkingDirectoryRoot;
#[derive(Debug)]
pub struct AgentWorkingDirectoryRoot;
#[derive(Debug)]
pub struct GitWorkingDirectoryRoot;
#[derive(Debug)]
pub struct TemporaryRoot<O>(PhantomData<O>);

#[derive(Debug)]
pub struct ReadOnly;
#[derive(Debug)]
pub struct Mutate;

pub trait MutationAllowed {}

impl MutationAllowed for MachineConfigRoot {}
impl MutationAllowed for DestinationRepositoryRoot {}
impl MutationAllowed for RunRecordRoot {}
impl<O> MutationAllowed for TemporaryRoot<O> {}

#[derive(Debug)]
pub struct AuthorityRoot<K, A> {
    pub(crate) handle: std::fs::File,
    pub(crate) identity: AuthorityIdentity,
    pub(crate) display_path: AbsolutePath,
    pub(crate) _kind: PhantomData<K>,
    pub(crate) _access: PhantomData<A>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorityAcceptance {
    pub(crate) owner: AuthorityAdapterKind,
    pub(crate) root: AbsolutePath,
    pub(crate) identity: AuthorityIdentity,
}

impl AuthorityAcceptance {
    pub fn owner(&self) -> AuthorityAdapterKind {
        self.owner
    }

    pub fn root_path(&self) -> &AbsolutePath {
        &self.root
    }

    pub fn root_identity(&self) -> AuthorityIdentity {
        self.identity
    }

    pub fn accept_read_target(&self, target: &ReadTarget) -> Result<(), PathError> {
        self.accept_identity(target.root_identity())
    }

    pub fn accept_mutation_target(&self, target: &MutationTarget) -> Result<(), PathError> {
        self.accept_identity(target.root_identity())
    }

    fn accept_identity(&self, actual: AuthorityIdentity) -> Result<(), PathError> {
        if actual == self.identity {
            return Ok(());
        }
        Err(PathError::AuthorityMismatch {
            owner: self.owner,
            root: self.root.as_path().display().to_string(),
            expected: self.identity,
            actual,
        })
    }
}

pub trait AuthorityAdapter {
    fn authority_acceptance(&self) -> &AuthorityAcceptance;

    fn accept_read_target(&self, target: &ReadTarget) -> Result<(), PathError> {
        self.authority_acceptance().accept_read_target(target)
    }

    fn accept_mutation_target(&self, target: &MutationTarget) -> Result<(), PathError> {
        self.authority_acceptance().accept_mutation_target(target)
    }
}

impl<K, A> AuthorityRoot<K, A> {
    pub fn acceptance(&self, owner: AuthorityAdapterKind) -> AuthorityAcceptance {
        AuthorityAcceptance {
            owner,
            root: self.display_path.clone(),
            identity: self.identity,
        }
    }
}

#[cfg(test)]
impl<K, A> AuthorityRoot<K, A> {
    pub(crate) fn test_handle_mut(&mut self) -> &mut std::fs::File {
        &mut self.handle
    }
}

impl<K> AuthorityRoot<K, ReadOnly> {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PathError> {
        backend::open_read_root(path.as_ref())
    }

    pub fn identity(&self) -> AuthorityIdentity {
        self.identity
    }

    pub fn display_path(&self) -> &AbsolutePath {
        &self.display_path
    }

    pub fn resolve_read(
        &self,
        path: &RelativePath,
        expected: ObjectClass,
    ) -> Result<ReadTarget, PathError> {
        backend::resolve_read(self, path, expected)
    }
}

impl<K: MutationAllowed> AuthorityRoot<K, Mutate> {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PathError> {
        backend::open_mutation_root(path.as_ref())
    }

    pub fn identity(&self) -> AuthorityIdentity {
        self.identity
    }

    pub fn display_path(&self) -> &AbsolutePath {
        &self.display_path
    }

    pub fn resolve_mutation(
        &self,
        path: &RelativePath,
        intent: MutationIntent,
    ) -> Result<MutationTarget, PathError> {
        backend::resolve_mutation(self, path, intent)
    }
}

pub fn open_read_root<K>(path: impl AsRef<Path>) -> Result<AuthorityRoot<K, ReadOnly>, PathError> {
    AuthorityRoot::<K, ReadOnly>::open(path)
}

pub fn open_mutation_root<K: MutationAllowed>(
    path: impl AsRef<Path>,
) -> Result<AuthorityRoot<K, Mutate>, PathError> {
    AuthorityRoot::<K, Mutate>::open(path)
}

pub struct ReadTarget {
    pub(crate) handle: std::fs::File,
    pub(crate) parent: std::fs::File,
    pub(crate) identity: AuthorityIdentity,
    pub(crate) root_identity: AuthorityIdentity,
    pub(crate) relative: RelativePath,
}

impl ReadTarget {
    pub fn identity(&self) -> AuthorityIdentity {
        self.identity
    }

    pub fn root_identity(&self) -> AuthorityIdentity {
        self.root_identity
    }

    pub fn relative_path(&self) -> &RelativePath {
        &self.relative
    }

    pub fn try_clone_file(&self) -> Result<std::fs::File, PathError> {
        self.handle.try_clone().map_err(|error| PathError::Io {
            operation: "clone read handle".to_owned(),
            path: self.relative.display(),
            kind: error.to_string(),
            code: error.raw_os_error(),
        })
    }

    pub fn parent_identity(&self) -> Result<AuthorityIdentity, PathError> {
        backend::identity_for_file(&self.parent, &self.relative.display())
    }
}

pub struct MutationTarget {
    pub(crate) handle: Option<std::fs::File>,
    pub(crate) parent: std::fs::File,
    pub(crate) name: Vec<u8>,
    pub(crate) identity: Option<AuthorityIdentity>,
    pub(crate) root_identity: AuthorityIdentity,
    pub(crate) root_path: AbsolutePath,
    pub(crate) ancestor_identities: Vec<AuthorityIdentity>,
    pub(crate) relative: RelativePath,
    pub(crate) intent: MutationIntent,
}

impl MutationTarget {
    pub fn identity(&self) -> Option<AuthorityIdentity> {
        self.identity
    }

    pub fn root_identity(&self) -> AuthorityIdentity {
        self.root_identity
    }

    pub fn relative_path(&self) -> &RelativePath {
        &self.relative
    }

    pub fn intent(&self) -> MutationIntent {
        self.intent
    }

    pub fn revalidate(&self) -> Result<(), PathError> {
        backend::revalidate_mutation(self).map(|_| ())
    }

    pub fn into_file(self) -> Result<std::fs::File, PathError> {
        self.revalidate()?;
        self.handle.ok_or_else(|| PathError::NotFound {
            path: self.relative.display(),
        })
    }

    pub fn create_exclusive(self) -> Result<std::fs::File, PathError> {
        backend::create_exclusive(self)
    }

    pub(crate) fn create_exclusive_with_mode(self, mode: u32) -> Result<std::fs::File, PathError> {
        backend::create_exclusive_with_mode(self, mode)
    }

    pub(crate) fn clone_parent(&self) -> Result<std::fs::File, PathError> {
        self.parent.try_clone().map_err(|error| PathError::Io {
            operation: "clone mutation parent".to_owned(),
            path: self.relative.display(),
            kind: error.to_string(),
            code: error.raw_os_error(),
        })
    }
}

#[derive(Default)]
pub struct AuthorityRegistry {
    pub(crate) entries: HashMap<AuthorityIdentity, String>,
}

impl AuthorityRegistry {
    pub fn register_root<K, A>(
        &mut self,
        root: &AuthorityRoot<K, A>,
        label: impl Into<String>,
    ) -> Result<(), PathError> {
        self.register_identity(root.identity, label.into())
    }

    pub fn register_read_target(
        &mut self,
        target: &ReadTarget,
        label: impl Into<String>,
    ) -> Result<(), PathError> {
        self.register_identity(target.identity, label.into())
    }

    pub fn register_mutation_target(
        &mut self,
        target: &MutationTarget,
        label: impl Into<String>,
    ) -> Result<(), PathError> {
        if let Some(identity) = target.identity {
            self.register_identity(identity, label.into())
        } else {
            Err(PathError::AuthorityOverlap {
                path: target.relative.display(),
            })
        }
    }

    fn register_identity(
        &mut self,
        identity: AuthorityIdentity,
        label: String,
    ) -> Result<(), PathError> {
        if let Some(existing) = self.entries.get(&identity) {
            return Err(PathError::DuplicateAuthority {
                label,
                existing: existing.clone(),
                identity,
            });
        }
        self.entries.insert(identity, label);
        Ok(())
    }

    pub fn contains(&self, identity: AuthorityIdentity) -> bool {
        self.entries.contains_key(&identity)
    }
}

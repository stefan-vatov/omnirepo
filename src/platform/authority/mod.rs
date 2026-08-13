//! Authority-root and no-follow filesystem primitives.
//!
//! This module is deliberately independent from the legacy path helpers.  A
//! caller must first open a typed authority root and then resolve a validated
//! [`RelativePath`] through that root.  The returned handle owns the checked
//! object, so a later caller cannot replace it with a path from another root.

use std::{
    collections::HashMap,
    error::Error,
    fmt,
    marker::PhantomData,
    path::{Component, Path, PathBuf},
};

#[cfg(test)]
mod tests;

#[cfg(test)]
use std::cell::Cell;

#[cfg(test)]
thread_local! {
    static TEST_CREATION_MODE: Cell<u32> = const { Cell::new(0) };
    static TEST_DURABILITY_PHASE: Cell<u8> = const { Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn test_reset_observations() {
    TEST_CREATION_MODE.with(|value| value.set(0));
    TEST_DURABILITY_PHASE.with(|value| value.set(0));
}

#[cfg(test)]
pub(crate) fn test_creation_mode() -> u32 {
    TEST_CREATION_MODE.with(Cell::get)
}

#[cfg(test)]
pub(crate) fn test_durability_phase() -> u8 {
    TEST_DURABILITY_PHASE.with(Cell::get)
}

/// Synchronize a newly created file through the platform-owned backend.
pub(crate) fn sync_file(file: &std::fs::File, path: &str) -> Result<(), PathError> {
    backend::sync_file(file, path)
}

/// Synchronize a containing directory through the platform-owned backend.
pub(crate) fn sync_directory(directory: &std::fs::File, path: &str) -> Result<(), PathError> {
    backend::sync_directory(directory, path)
}

#[cfg(all(test, unix))]
fn observe_creation_mode(file: &std::fs::File) {
    use std::os::unix::fs::PermissionsExt;

    let mode = file
        .metadata()
        .expect("test mode observation reads the new file")
        .permissions()
        .mode()
        & 0o777;
    TEST_CREATION_MODE.with(|value| value.set(mode));
}

#[cfg(any(not(test), not(unix)))]
fn observe_creation_mode(_file: &std::fs::File) {}

#[cfg(test)]
fn mark_file_sync_complete() {
    TEST_DURABILITY_PHASE.with(|value| value.set(1));
}

#[cfg(not(test))]
fn mark_file_sync_complete() {}

#[cfg(test)]
fn mark_directory_sync_complete() {
    TEST_DURABILITY_PHASE.with(|value| {
        assert_eq!(value.get(), 1, "directory sync must follow file sync");
        value.set(2);
    });
}

#[cfg(not(test))]
fn mark_directory_sync_complete() {}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FilesystemKind {
    LinuxExtFamily,
    MacOsApfs,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FilesystemIdentity {
    pub(crate) device: u64,
    pub(crate) kind: FilesystemKind,
    pub(crate) mount_id: u64,
}

impl FilesystemIdentity {
    pub fn device(self) -> u64 {
        self.device
    }

    pub fn kind(self) -> FilesystemKind {
        self.kind
    }

    pub fn mount_id(self) -> u64 {
        self.mount_id
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ObjectIdentity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
}

impl ObjectIdentity {
    pub fn device(self) -> u64 {
        self.device
    }

    pub fn inode(self) -> u64 {
        self.inode
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AuthorityIdentity {
    pub(crate) filesystem: FilesystemIdentity,
    pub(crate) object: ObjectIdentity,
}

impl AuthorityIdentity {
    pub fn filesystem(self) -> FilesystemIdentity {
        self.filesystem
    }

    pub fn object(self) -> ObjectIdentity {
        self.object
    }
}

impl fmt::Display for AuthorityIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "device={} inode={} filesystem={:?} mount={}",
            self.object.device, self.object.inode, self.filesystem.kind, self.filesystem.mount_id
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AuthorityAdapterKind {
    Configuration,
    Source,
    Record,
    Process,
    Agent,
    Git,
}

impl fmt::Display for AuthorityAdapterKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Configuration => "configuration",
            Self::Source => "source",
            Self::Record => "run-record",
            Self::Process => "process",
            Self::Agent => "agent",
            Self::Git => "git",
        };
        formatter.write_str(name)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PathError {
    UnsupportedPlatform,
    UnsupportedFilesystem {
        path: String,
        kind: String,
    },
    InvalidAuthorityRoot {
        path: String,
        reason: String,
    },
    InvalidAbsolutePath {
        path: String,
        reason: String,
    },
    InvalidRelativePath {
        input: String,
        reason: String,
    },
    NotFound {
        path: String,
    },
    LinkLikeObject {
        path: String,
    },
    MountCrossing {
        path: String,
    },
    UnsupportedObject {
        path: String,
        expected: ObjectClass,
    },
    UnsafeHardLink {
        path: String,
        links: u64,
    },
    ConcurrentReplacement {
        path: String,
        reason: String,
    },
    AuthorityMismatch {
        owner: AuthorityAdapterKind,
        root: String,
        expected: AuthorityIdentity,
        actual: AuthorityIdentity,
    },
    DuplicateAuthority {
        label: String,
        existing: String,
        identity: AuthorityIdentity,
    },
    AuthorityOverlap {
        path: String,
    },
    Io {
        operation: String,
        path: String,
        kind: String,
        code: Option<i32>,
    },
}

impl fmt::Display for PathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => {
                write!(
                    formatter,
                    "unsupported platform: only Linux and macOS are supported"
                )
            }
            Self::UnsupportedFilesystem { path, kind } => write!(
                formatter,
                "unsupported filesystem at {path:?}: {kind}; only local ext-family or APFS is supported"
            ),
            Self::InvalidAuthorityRoot { path, reason } => {
                write!(formatter, "invalid authority root {path:?}: {reason}")
            }
            Self::InvalidAbsolutePath { path, reason } => {
                write!(formatter, "invalid absolute path {path:?}: {reason}")
            }
            Self::InvalidRelativePath { input, reason } => {
                write!(formatter, "invalid relative path {input:?}: {reason}")
            }
            Self::NotFound { path } => write!(formatter, "authority path not found: {path:?}"),
            Self::LinkLikeObject { path } => {
                write!(
                    formatter,
                    "link-like object rejected without following: {path:?}"
                )
            }
            Self::MountCrossing { path } => {
                write!(
                    formatter,
                    "filesystem boundary crossed below authority root: {path:?}"
                )
            }
            Self::UnsupportedObject { path, expected } => {
                write!(
                    formatter,
                    "unsupported object at {path:?}; expected {expected:?}"
                )
            }
            Self::UnsafeHardLink { path, links } => write!(
                formatter,
                "mutation target {path:?} has {links} hard-link names and is unsafe"
            ),
            Self::ConcurrentReplacement { path, reason } => write!(
                formatter,
                "authority path was replaced while a mutation was in flight: {path:?}: {reason}"
            ),
            Self::AuthorityMismatch {
                owner,
                root,
                expected,
                actual,
            } => write!(
                formatter,
                "{owner} adapter target is outside owning root {root:?}: expected {expected}, got {actual}"
            ),
            Self::DuplicateAuthority {
                label,
                existing,
                identity,
            } => write!(
                formatter,
                "authority {label:?} duplicates {existing:?} ({identity})"
            ),
            Self::AuthorityOverlap { path } => {
                write!(formatter, "undeclared authority overlap at {path:?}")
            }
            Self::Io {
                operation,
                path,
                kind,
                code,
            } => write!(
                formatter,
                "{operation} failed for authority path {path:?}: {kind} (errno={code:?})"
            ),
        }
    }
}

impl Error for PathError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectClass {
    Any,
    Directory,
    RegularFile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationIntent {
    CreateExclusive,
    Replace,
    Append,
    Remove,
    Rename,
}

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

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct RelativePath {
    components: Vec<Vec<u8>>,
}

impl RelativePath {
    pub fn root() -> Self {
        Self {
            components: Vec::new(),
        }
    }

    pub fn parse(input: &str) -> Result<Self, PathError> {
        if input.is_empty() {
            return Err(PathError::InvalidRelativePath {
                input: input.to_owned(),
                reason: "an empty path is reserved for RelativePath::root()".to_owned(),
            });
        }
        if input.as_bytes().contains(&0) {
            return Err(PathError::InvalidRelativePath {
                input: input.to_owned(),
                reason: "NUL is not a path component".to_owned(),
            });
        }
        if input.starts_with('/') {
            return Err(PathError::InvalidRelativePath {
                input: input.to_owned(),
                reason: "absolute paths are not valid nested references".to_owned(),
            });
        }

        let mut components = Vec::new();
        for component in input.split('/') {
            if component.is_empty() || component == "." {
                continue;
            }
            if component == ".." {
                return Err(PathError::InvalidRelativePath {
                    input: input.to_owned(),
                    reason: "parent-directory traversal is not allowed".to_owned(),
                });
            }
            components.push(component.as_bytes().to_vec());
        }

        if components.is_empty() {
            return Err(PathError::InvalidRelativePath {
                input: input.to_owned(),
                reason: "the path has no component; use RelativePath::root() explicitly".to_owned(),
            });
        }

        Ok(Self { components })
    }

    pub fn components(&self) -> impl Iterator<Item = &[u8]> {
        self.components.iter().map(Vec::as_slice)
    }

    pub(crate) fn display(&self) -> String {
        self.components
            .iter()
            .map(|component| String::from_utf8_lossy(component))
            .collect::<Vec<_>>()
            .join("/")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AbsolutePath {
    path: PathBuf,
}

impl AbsolutePath {
    pub fn parse(input: &str) -> Result<Self, PathError> {
        let path = PathBuf::from(input);
        Self::from_path(&path)
    }

    pub fn from_path(path: &Path) -> Result<Self, PathError> {
        if !path.is_absolute() {
            return Err(PathError::InvalidAbsolutePath {
                path: path.display().to_string(),
                reason: "an authority root must be absolute".to_owned(),
            });
        }
        let Some(text) = path.to_str() else {
            return Err(PathError::InvalidAbsolutePath {
                path: path.display().to_string(),
                reason: "authority paths must be UTF-8".to_owned(),
            });
        };
        if text.as_bytes().contains(&0) {
            return Err(PathError::InvalidAbsolutePath {
                path: path.display().to_string(),
                reason: "NUL is not a path component".to_owned(),
            });
        }
        for component in path.components() {
            match component {
                Component::ParentDir => {
                    return Err(PathError::InvalidAbsolutePath {
                        path: path.display().to_string(),
                        reason: "parent-directory traversal is not allowed".to_owned(),
                    });
                }
                Component::Prefix(_) => {
                    return Err(PathError::InvalidAbsolutePath {
                        path: path.display().to_string(),
                        reason: "platform prefixes and device paths are not supported".to_owned(),
                    });
                }
                Component::Normal(value) if value.to_str().is_none() => {
                    return Err(PathError::InvalidAbsolutePath {
                        path: path.display().to_string(),
                        reason: "authority paths must be UTF-8".to_owned(),
                    });
                }
                Component::RootDir | Component::CurDir | Component::Normal(_) => {}
            }
        }
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    pub fn as_path(&self) -> &Path {
        &self.path
    }
}

#[derive(Debug)]
pub struct AuthorityRoot<K, A> {
    handle: std::fs::File,
    identity: AuthorityIdentity,
    display_path: AbsolutePath,
    _kind: PhantomData<K>,
    _access: PhantomData<A>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorityAcceptance {
    owner: AuthorityAdapterKind,
    root: AbsolutePath,
    identity: AuthorityIdentity,
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
    parent: std::fs::File,
    identity: AuthorityIdentity,
    root_identity: AuthorityIdentity,
    relative: RelativePath,
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
    handle: Option<std::fs::File>,
    parent: std::fs::File,
    name: Vec<u8>,
    identity: Option<AuthorityIdentity>,
    root_identity: AuthorityIdentity,
    root_path: AbsolutePath,
    ancestor_identities: Vec<AuthorityIdentity>,
    relative: RelativePath,
    intent: MutationIntent,
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
    entries: HashMap<AuthorityIdentity, String>,
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod backend {
    use super::*;
    use std::{
        ffi::{CString, c_char, c_int},
        io,
        os::{
            fd::{AsRawFd, FromRawFd, OwnedFd},
            unix::fs::MetadataExt,
        },
    };

    #[cfg(target_os = "linux")]
    const AT_FDCWD: c_int = -100;
    #[cfg(target_os = "macos")]
    const AT_FDCWD: c_int = -2;
    #[cfg(target_os = "linux")]
    const AT_EMPTY_PATH: c_int = 0x1000;

    #[cfg(target_os = "linux")]
    const O_CLOEXEC: c_int = 0x80000;
    #[cfg(target_os = "macos")]
    const O_CLOEXEC: c_int = 0x01000000;
    #[cfg(target_os = "linux")]
    const O_NOFOLLOW: c_int = 0x20000;
    #[cfg(target_os = "macos")]
    const O_NOFOLLOW: c_int = 0x00000100;
    #[cfg(target_os = "linux")]
    const O_NONBLOCK: c_int = 0x800;
    #[cfg(target_os = "macos")]
    const O_NONBLOCK: c_int = 0x00000004;
    const O_RDONLY: c_int = 0;
    const O_RDWR: c_int = 2;
    #[cfg(target_os = "linux")]
    const O_CREAT: c_int = 0x40;
    #[cfg(target_os = "macos")]
    const O_CREAT: c_int = 0x00000200;
    #[cfg(target_os = "linux")]
    const O_EXCL: c_int = 0x80;
    #[cfg(target_os = "macos")]
    const O_EXCL: c_int = 0x00000800;
    const F_SETFD: c_int = 2;
    const FD_CLOEXEC: c_int = 1;
    const EACCES: i32 = 13;
    const EEXIST: i32 = 17;
    #[cfg(target_os = "linux")]
    const ELOOP: i32 = 40;
    #[cfg(target_os = "macos")]
    const ELOOP: i32 = 62;
    const ENOENT: i32 = 2;
    const ENOTDIR: i32 = 20;
    const EISDIR: i32 = 21;
    const EPERM: i32 = 1;
    #[cfg(target_os = "linux")]
    const STATX_MNT_ID: u32 = 0x1000;

    #[cfg(target_os = "linux")]
    #[repr(C)]
    struct StatFs {
        f_type: i64,
        f_bsize: i64,
        f_blocks: u64,
        f_bfree: u64,
        f_bavail: u64,
        f_files: u64,
        f_ffree: u64,
        f_fsid: [i32; 2],
        f_namelen: i64,
        f_frsize: i64,
        f_flags: i64,
        f_spare: [i64; 4],
    }

    #[cfg(target_os = "macos")]
    #[repr(C)]
    struct StatFs {
        f_bsize: u32,
        f_iosize: i32,
        f_blocks: u64,
        f_bfree: u64,
        f_bavail: u64,
        f_files: u64,
        f_ffree: u64,
        f_fsid: [i32; 2],
        f_owner: u32,
        f_type: u32,
        f_flags: u32,
        f_fssubtype: u32,
        f_fstypename: [c_char; 16],
        f_mntonname: [c_char; 1024],
        f_mntfromname: [c_char; 1024],
        f_reserved: [u32; 8],
    }

    #[cfg(target_os = "linux")]
    #[repr(C)]
    struct RawStatx {
        bytes: [u8; 256],
    }

    unsafe extern "C" {
        fn openat(dirfd: c_int, pathname: *const c_char, flags: c_int, ...) -> c_int;
        fn fstatfs(fd: c_int, buffer: *mut StatFs) -> c_int;
        fn dup(fd: c_int) -> c_int;
        fn fcntl(fd: c_int, command: c_int, ...) -> c_int;
        #[cfg(target_os = "linux")]
        fn statx(
            dirfd: c_int,
            pathname: *const c_char,
            flags: c_int,
            mask: u32,
            buffer: *mut RawStatx,
        ) -> c_int;
    }

    pub fn open_read_root<K>(path: &Path) -> Result<AuthorityRoot<K, ReadOnly>, PathError> {
        open_root(path)
    }

    pub fn open_mutation_root<K: MutationAllowed>(
        path: &Path,
    ) -> Result<AuthorityRoot<K, Mutate>, PathError> {
        open_root(path)
    }

    fn open_root<K, A>(path: &Path) -> Result<AuthorityRoot<K, A>, PathError> {
        let display_path = AbsolutePath::from_path(path)?;
        let mut current = open_at(
            AT_FDCWD,
            b"/",
            O_RDONLY | O_CLOEXEC | O_NOFOLLOW | O_NONBLOCK,
            0,
        )
        .map_err(|error| map_io("open authority filesystem root", "/", error))?;

        for component in absolute_components(display_path.as_path())? {
            let next = open_at(
                current.as_raw_fd(),
                &component,
                O_RDONLY | O_CLOEXEC | O_NOFOLLOW | O_NONBLOCK,
                0,
            )
            .map_err(|error| {
                map_io(
                    "open authority root component",
                    &display_path_string(&component),
                    error,
                )
            })?;
            let metadata = next.metadata().map_err(|error| {
                map_io(
                    "inspect authority root component",
                    &display_path_string(&component),
                    error,
                )
            })?;
            if !metadata.is_dir() {
                return Err(PathError::InvalidAuthorityRoot {
                    path: display_path.as_path().display().to_string(),
                    reason: "every authority root component must be a directory".to_owned(),
                });
            }
            current = next;
        }

        let identity = identity_for_file(&current, &display_path.as_path().display().to_string())?;
        Ok(AuthorityRoot {
            handle: current,
            identity,
            display_path,
            _kind: PhantomData,
            _access: PhantomData,
        })
    }

    pub fn resolve_read<K>(
        root: &AuthorityRoot<K, ReadOnly>,
        path: &RelativePath,
        expected: ObjectClass,
    ) -> Result<ReadTarget, PathError> {
        let (parent, name, _) = walk_parent(root, path)?;
        let (handle, identity) = if let Some(name) = name {
            let handle = open_at(
                parent.as_raw_fd(),
                &name,
                O_RDONLY | O_CLOEXEC | O_NOFOLLOW | O_NONBLOCK,
                0,
            )
            .map_err(|error| map_io("open read target", &path.display(), error))?;
            let identity = validate_target(root, &handle, path, expected)?;
            (handle, identity)
        } else {
            let handle = duplicate(&root.handle).map_err(|error| {
                map_io("duplicate authority root handle", &path.display(), error)
            })?;
            let identity = validate_target(root, &handle, path, ObjectClass::Directory)?;
            (handle, identity)
        };

        Ok(ReadTarget {
            handle,
            parent,
            identity,
            root_identity: root.identity,
            relative: path.clone(),
        })
    }

    pub fn resolve_mutation<K: MutationAllowed>(
        root: &AuthorityRoot<K, Mutate>,
        path: &RelativePath,
        intent: MutationIntent,
    ) -> Result<MutationTarget, PathError> {
        if path.components.is_empty() {
            return Err(PathError::UnsupportedObject {
                path: path.display(),
                expected: ObjectClass::RegularFile,
            });
        }

        let (parent, name, ancestor_identities) = walk_parent(root, path)?;
        let name = name.expect("non-empty relative path has a leaf");
        let flags = O_RDWR | O_CLOEXEC | O_NOFOLLOW | O_NONBLOCK;
        match open_at(parent.as_raw_fd(), &name, flags, 0) {
            Ok(handle) => {
                let identity = validate_target(root, &handle, path, ObjectClass::RegularFile)?;
                let metadata =
                    inspect_metadata(&handle, "inspect mutation target", &path.display())?;
                reject_unsafe_hard_link(&path.display(), &metadata)?;
                Ok(MutationTarget {
                    handle: Some(handle),
                    parent,
                    name,
                    identity: Some(identity),
                    root_identity: root.identity,
                    root_path: root.display_path.clone(),
                    ancestor_identities,
                    relative: path.clone(),
                    intent,
                })
            }
            Err(error)
                if error.raw_os_error() == Some(ENOENT)
                    && intent == MutationIntent::CreateExclusive =>
            {
                Ok(MutationTarget {
                    handle: None,
                    parent,
                    name,
                    identity: None,
                    root_identity: root.identity,
                    root_path: root.display_path.clone(),
                    ancestor_identities,
                    relative: path.clone(),
                    intent,
                })
            }
            Err(error) => Err(map_io("open mutation target", &path.display(), error)),
        }
    }

    pub fn revalidate_mutation(target: &MutationTarget) -> Result<std::fs::File, PathError> {
        let path = target.relative.display();
        let root = open_root::<DestinationRepositoryRoot, Mutate>(target.root_path.as_path())
            .map_err(|error| replacement_error(&path, "authority root", error))?;
        if root.identity != target.root_identity {
            return Err(replacement_error(
                &path,
                "authority root identity changed",
                "the declared root now names a different object",
            ));
        }

        // `target.name` is private and is always the final component returned
        // by `walk_parent` for `target.relative`. Rewalking the same relative
        // path cannot produce a different leaf name, so no safe caller can
        // reach a separate name-divergence state here.
        let (parent, _, ancestor_identities) = walk_parent(&root, &target.relative)
            .map_err(|error| replacement_error(&path, "authority ancestor", error))?;
        if ancestor_identities != target.ancestor_identities {
            return Err(replacement_error(
                &path,
                "authority ancestor identity changed",
                "an ancestor was renamed or replaced",
            ));
        }

        if let Some(expected) = target.identity {
            let handle = open_at(
                parent.as_raw_fd(),
                &target.name,
                O_RDWR | O_CLOEXEC | O_NOFOLLOW | O_NONBLOCK,
                0,
            )
            .map_err(|error| {
                replacement_error(
                    &path,
                    "authority leaf lookup",
                    map_io("revalidate mutation target", &path, error),
                )
            })?;
            let identity =
                validate_target_identity(root.identity, &handle, &path, ObjectClass::RegularFile)
                    .map_err(|error| replacement_error(&path, "authority leaf identity", error))?;
            if identity != expected {
                return Err(replacement_error(
                    &path,
                    "authority leaf identity changed",
                    "the leaf now names a different object",
                ));
            }
            let metadata = inspect_metadata(&handle, "inspect mutation target", &path)
                .map_err(|error| replacement_error(&path, "authority leaf metadata", error))?;
            reject_unsafe_hard_link(&path, &metadata)?;
        } else {
            match open_at(
                parent.as_raw_fd(),
                &target.name,
                O_RDONLY | O_CLOEXEC | O_NOFOLLOW | O_NONBLOCK,
                0,
            ) {
                Ok(handle) => {
                    drop(handle);
                    return Err(replacement_error(
                        &path,
                        "authority leaf appeared",
                        "a create candidate is no longer absent",
                    ));
                }
                Err(error) if error.raw_os_error() == Some(ENOENT) => {}
                Err(error) => {
                    return Err(replacement_error(
                        &path,
                        "authority create-candidate lookup",
                        map_io("revalidate create candidate", &path, error),
                    ));
                }
            }
        }

        Ok(parent)
    }

    pub fn create_exclusive(target: MutationTarget) -> Result<std::fs::File, PathError> {
        create_exclusive_with_mode(target, 0o644)
    }

    pub(crate) fn create_exclusive_with_mode(
        target: MutationTarget,
        mode: u32,
    ) -> Result<std::fs::File, PathError> {
        if target.intent != MutationIntent::CreateExclusive {
            return Err(PathError::Io {
                operation: "create exclusive target".to_owned(),
                path: target.relative.display(),
                kind: "mutation intent is not CreateExclusive".to_owned(),
                code: None,
            });
        }
        let parent = revalidate_mutation(&target)?;
        if target.handle.is_some() {
            return Err(PathError::Io {
                operation: "create exclusive target".to_owned(),
                path: target.relative.display(),
                kind: "target already exists".to_owned(),
                code: Some(EEXIST),
            });
        }
        let handle = open_at(
            parent.as_raw_fd(),
            &target.name,
            O_RDWR | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW | O_NONBLOCK,
            mode,
        )
        .map_err(|error| map_io("create mutation target", &target.relative.display(), error))?;
        observe_creation_mode(&handle);
        validate_target_identity(
            target.root_identity,
            &handle,
            &target.relative.display(),
            ObjectClass::RegularFile,
        )?;
        let metadata = inspect_metadata(
            &handle,
            "inspect created mutation target",
            &target.relative.display(),
        )?;
        reject_unsafe_hard_link(&target.relative.display(), &metadata)?;
        // `validate_target_identity` above already compares the opened file's
        // device with the root device. Its returned object identity stores that
        // same metadata device, so a second post-create device check cannot
        // observe a new state.
        Ok(handle)
    }

    pub(crate) fn sync_file(file: &std::fs::File, path: &str) -> Result<(), PathError> {
        file.sync_all()
            .map_err(|error| map_io("sync mutation file", path, error))?;
        mark_file_sync_complete();
        Ok(())
    }

    pub(crate) fn sync_directory(directory: &std::fs::File, path: &str) -> Result<(), PathError> {
        directory
            .sync_all()
            .map_err(|error| map_io("sync mutation directory", path, error))?;
        mark_directory_sync_complete();
        Ok(())
    }

    pub(crate) fn inspect_metadata(
        file: &std::fs::File,
        operation: &str,
        path: &str,
    ) -> Result<std::fs::Metadata, PathError> {
        file.metadata()
            .map_err(|error| map_io(operation, path, error))
    }

    pub(crate) fn reject_unsafe_hard_link(
        path: &str,
        metadata: &std::fs::Metadata,
    ) -> Result<(), PathError> {
        if metadata.nlink() > 1 {
            return Err(PathError::UnsafeHardLink {
                path: path.to_owned(),
                links: metadata.nlink(),
            });
        }
        Ok(())
    }

    fn replacement_error(path: &str, boundary: &str, detail: impl fmt::Display) -> PathError {
        PathError::ConcurrentReplacement {
            path: path.to_owned(),
            reason: format!("{boundary}: {detail}"),
        }
    }

    type ParentWalk = (std::fs::File, Option<Vec<u8>>, Vec<AuthorityIdentity>);

    fn walk_parent<K, A>(
        root: &AuthorityRoot<K, A>,
        path: &RelativePath,
    ) -> Result<ParentWalk, PathError> {
        let mut current = duplicate(&root.handle)
            .map_err(|error| map_io("duplicate authority root handle", &path.display(), error))?;
        let mut ancestor_identities = Vec::new();
        let mut components = path.components.iter();
        let Some(first) = components.next() else {
            return Ok((current, None, ancestor_identities));
        };
        let mut previous = first.clone();
        for component in components {
            let next = open_at(
                current.as_raw_fd(),
                &previous,
                O_RDONLY | O_CLOEXEC | O_NOFOLLOW | O_NONBLOCK,
                0,
            )
            .map_err(|error| map_io("open authority parent", &path.display(), error))?;
            let identity = validate_target_identity(
                root.identity,
                &next,
                &path.display(),
                ObjectClass::Directory,
            )?;
            ancestor_identities.push(identity);
            let metadata = next
                .metadata()
                .map_err(|error| map_io("inspect authority parent", &path.display(), error))?;
            if !metadata.is_dir() {
                return Err(PathError::UnsupportedObject {
                    path: path.display(),
                    expected: ObjectClass::Directory,
                });
            }
            current = next;
            previous = component.clone();
        }
        Ok((current, Some(previous), ancestor_identities))
    }

    fn validate_target<K, A>(
        root: &AuthorityRoot<K, A>,
        handle: &std::fs::File,
        path: &RelativePath,
        expected: ObjectClass,
    ) -> Result<AuthorityIdentity, PathError> {
        validate_target_identity(root.identity, handle, &path.display(), expected)
    }

    pub(crate) fn validate_target_identity(
        root_identity: AuthorityIdentity,
        handle: &std::fs::File,
        path: &str,
        expected: ObjectClass,
    ) -> Result<AuthorityIdentity, PathError> {
        let metadata = handle
            .metadata()
            .map_err(|error| map_io("inspect authority target", path, error))?;
        if metadata.dev() != root_identity.filesystem.device {
            return Err(PathError::MountCrossing {
                path: path.to_owned(),
            });
        }
        let identity = identity_for_file(handle, path)?;
        if identity.filesystem != root_identity.filesystem {
            return Err(PathError::MountCrossing {
                path: path.to_owned(),
            });
        }
        let class_matches = match expected {
            ObjectClass::Any => metadata.is_file() || metadata.is_dir(),
            ObjectClass::Directory => metadata.is_dir(),
            ObjectClass::RegularFile => metadata.is_file(),
        };
        if !class_matches {
            return Err(PathError::UnsupportedObject {
                path: path.to_owned(),
                expected,
            });
        }
        Ok(identity)
    }

    pub fn identity_for_file(
        file: &std::fs::File,
        path: &str,
    ) -> Result<AuthorityIdentity, PathError> {
        let metadata = file
            .metadata()
            .map_err(|error| map_io("read filesystem identity", path, error))?;
        let filesystem = filesystem_identity(file, metadata.dev(), path)?;
        Ok(AuthorityIdentity {
            filesystem,
            object: ObjectIdentity {
                device: metadata.dev(),
                inode: metadata.ino(),
            },
        })
    }

    pub(crate) fn filesystem_identity(
        file: &std::fs::File,
        device: u64,
        path: &str,
    ) -> Result<FilesystemIdentity, PathError> {
        let mut stat = std::mem::MaybeUninit::<StatFs>::uninit();
        let result = unsafe { fstatfs(file.as_raw_fd(), stat.as_mut_ptr()) };
        if result != 0 {
            return Err(map_io(
                "read filesystem type",
                path,
                io::Error::last_os_error(),
            ));
        }
        let stat = unsafe { stat.assume_init() };
        #[cfg(target_os = "linux")]
        let kind = if stat.f_type == 0xEF53 {
            FilesystemKind::LinuxExtFamily
        } else {
            return Err(PathError::UnsupportedFilesystem {
                path: path.to_owned(),
                kind: format!("Linux filesystem magic 0x{:x}", stat.f_type),
            });
        };
        #[cfg(target_os = "macos")]
        let kind = {
            let bytes = stat
                .f_fstypename
                .iter()
                .map(|value| *value as u8)
                .take_while(|value| *value != 0)
                .collect::<Vec<_>>();
            if bytes == b"apfs" {
                FilesystemKind::MacOsApfs
            } else {
                return Err(PathError::UnsupportedFilesystem {
                    path: path.to_owned(),
                    kind: String::from_utf8_lossy(&bytes).into_owned(),
                });
            }
        };
        let mount_id = mount_id_for_file(file, path)?;
        Ok(FilesystemIdentity {
            device,
            kind,
            mount_id,
        })
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn mount_id_for_file(file: &std::fs::File, path: &str) -> Result<u64, PathError> {
        let empty = CString::new("").expect("empty CString has no NUL");
        let mut stat = std::mem::MaybeUninit::<RawStatx>::zeroed();
        let result = unsafe {
            statx(
                file.as_raw_fd(),
                empty.as_ptr(),
                AT_EMPTY_PATH,
                STATX_MNT_ID,
                stat.as_mut_ptr(),
            )
        };
        if result != 0 {
            return Err(map_io(
                "read mount identity",
                path,
                io::Error::last_os_error(),
            ));
        }
        let stat = unsafe { stat.assume_init() };
        let bytes = &stat.bytes[144..152];
        Ok(u64::from_ne_bytes(
            bytes.try_into().expect("mount ID has eight bytes"),
        ))
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn mount_id_for_file(file: &std::fs::File, path: &str) -> Result<u64, PathError> {
        let mut stat = std::mem::MaybeUninit::<StatFs>::uninit();
        let result = unsafe { fstatfs(file.as_raw_fd(), stat.as_mut_ptr()) };
        if result != 0 {
            return Err(map_io(
                "read mount identity",
                path,
                io::Error::last_os_error(),
            ));
        }
        let stat = unsafe { stat.assume_init() };
        let [high, low] = stat.f_fsid;
        Ok((u32::from_ne_bytes(high.to_ne_bytes()) as u64) << 32
            | u32::from_ne_bytes(low.to_ne_bytes()) as u64)
    }

    pub(crate) fn absolute_components(path: &Path) -> Result<Vec<Vec<u8>>, PathError> {
        let mut components = Vec::new();
        for component in path.components() {
            if let Component::Normal(value) = component {
                let text = value
                    .to_str()
                    .ok_or_else(|| PathError::InvalidAbsolutePath {
                        path: path.display().to_string(),
                        reason: "authority paths must be UTF-8".to_owned(),
                    })?;
                components.push(text.as_bytes().to_vec());
            }
        }
        Ok(components)
    }

    fn display_path_string(component: &[u8]) -> String {
        String::from_utf8_lossy(component).into_owned()
    }

    pub(crate) fn open_at(
        directory: c_int,
        name: &[u8],
        flags: c_int,
        mode: u32,
    ) -> Result<std::fs::File, io::Error> {
        let name = CString::new(name).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "NUL is not a path component")
        })?;
        let fd = unsafe { openat(directory, name.as_ptr(), flags, mode) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { std::fs::File::from_raw_fd(fd) })
    }

    pub(crate) fn duplicate(file: &std::fs::File) -> Result<std::fs::File, io::Error> {
        let fd = unsafe { dup(file.as_raw_fd()) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let result = unsafe { fcntl(fd, F_SETFD, FD_CLOEXEC) };
        if result < 0 {
            let _ = unsafe { OwnedFd::from_raw_fd(fd) };
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { std::fs::File::from_raw_fd(fd) })
    }

    pub(crate) fn map_io(operation: &str, path: &str, error: io::Error) -> PathError {
        match error.raw_os_error() {
            Some(ELOOP) => PathError::LinkLikeObject {
                path: path.to_owned(),
            },
            Some(ENOENT) => PathError::NotFound {
                path: path.to_owned(),
            },
            Some(ENOTDIR) => PathError::UnsupportedObject {
                path: path.to_owned(),
                expected: ObjectClass::Directory,
            },
            Some(EISDIR) => PathError::UnsupportedObject {
                path: path.to_owned(),
                expected: ObjectClass::RegularFile,
            },
            Some(EACCES | EPERM) => PathError::Io {
                operation: operation.to_owned(),
                path: path.to_owned(),
                kind: "permission denied".to_owned(),
                code: error.raw_os_error(),
            },
            _ => PathError::Io {
                operation: operation.to_owned(),
                path: path.to_owned(),
                kind: error.to_string(),
                code: error.raw_os_error(),
            },
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod backend {
    use super::*;

    pub fn open_read_root<K>(_path: &Path) -> Result<AuthorityRoot<K, ReadOnly>, PathError> {
        Err(PathError::UnsupportedPlatform)
    }

    pub fn open_mutation_root<K: MutationAllowed>(
        _path: &Path,
    ) -> Result<AuthorityRoot<K, Mutate>, PathError> {
        Err(PathError::UnsupportedPlatform)
    }

    pub fn resolve_read<K>(
        _root: &AuthorityRoot<K, ReadOnly>,
        _path: &RelativePath,
        _expected: ObjectClass,
    ) -> Result<ReadTarget, PathError> {
        Err(PathError::UnsupportedPlatform)
    }

    pub fn resolve_mutation<K: MutationAllowed>(
        _root: &AuthorityRoot<K, Mutate>,
        _path: &RelativePath,
        _intent: MutationIntent,
    ) -> Result<MutationTarget, PathError> {
        Err(PathError::UnsupportedPlatform)
    }

    pub fn revalidate_mutation(_target: &MutationTarget) -> Result<std::fs::File, PathError> {
        Err(PathError::UnsupportedPlatform)
    }

    pub fn create_exclusive(_target: MutationTarget) -> Result<std::fs::File, PathError> {
        Err(PathError::UnsupportedPlatform)
    }

    pub(crate) fn create_exclusive_with_mode(
        _target: MutationTarget,
        _mode: u32,
    ) -> Result<std::fs::File, PathError> {
        Err(PathError::UnsupportedPlatform)
    }

    pub(crate) fn sync_file(_file: &std::fs::File, _path: &str) -> Result<(), PathError> {
        Err(PathError::UnsupportedPlatform)
    }

    pub(crate) fn sync_directory(_directory: &std::fs::File, _path: &str) -> Result<(), PathError> {
        Err(PathError::UnsupportedPlatform)
    }

    pub fn identity_for_file(
        _file: &std::fs::File,
        _path: &str,
    ) -> Result<AuthorityIdentity, PathError> {
        Err(PathError::UnsupportedPlatform)
    }
}

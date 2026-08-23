use std::{error::Error, fmt};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FilesystemKind {
    Linux,
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
                "unsupported filesystem at {path:?}: {kind}; only APFS is supported on macOS"
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

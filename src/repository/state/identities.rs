use super::{DomainError, validate_text};
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ManagedSectionId(String);

impl ManagedSectionId {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        // One rule owner: the managed-content section-ID rule.
        if !crate::configuration::is_valid_section_id(&value) {
            return Err(DomainError::InvalidManagedSectionId { value });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RepositoryRoot {
    path: String,
    authority: AuthorityIdentity,
}

impl RepositoryRoot {
    pub fn new(
        value: impl Into<String>,
        authority: AuthorityIdentity,
    ) -> Result<Self, DomainError> {
        let value = value.into();
        if value.is_empty()
            || !value.starts_with('/')
            || value.as_bytes().contains(&0)
            || value.contains('\\')
            || (value.len() > 1 && value.ends_with('/'))
        {
            return Err(DomainError::InvalidAbsoluteRoot { value });
        }

        let mut components = value.split('/');
        let _root = components.next();
        if components.any(|component| component.is_empty() || component == "." || component == "..")
        {
            return Err(DomainError::InvalidAbsoluteRoot { value });
        }
        Ok(Self {
            path: value,
            authority,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.path
    }

    pub fn authority(&self) -> &AuthorityIdentity {
        &self.authority
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RelativePath {
    bytes: Vec<u8>,
}

impl RelativePath {
    pub fn new(value: impl AsRef<str>) -> Result<Self, DomainError> {
        Self::from_bytes(value.as_ref().as_bytes())
    }

    pub fn from_bytes(value: impl AsRef<[u8]>) -> Result<Self, DomainError> {
        let value = value.as_ref();
        if value.is_empty() || value[0] == b'/' || value.contains(&0) {
            return Err(DomainError::InvalidRelativePath {
                value: String::from_utf8_lossy(value).into_owned(),
            });
        }

        let mut components: Vec<&[u8]> = Vec::new();
        for component in value.split(|byte| *byte == b'/') {
            if component.is_empty() || component == b"." {
                continue;
            }
            if component == b".." {
                return Err(DomainError::InvalidRelativePath {
                    value: String::from_utf8_lossy(value).into_owned(),
                });
            }
            components.push(component);
        }
        if components.is_empty() {
            return Err(DomainError::InvalidRelativePath {
                value: String::from_utf8_lossy(value).into_owned(),
            });
        }

        let mut normalized = Vec::new();
        for (index, component) in components.iter().enumerate() {
            if index != 0 {
                normalized.push(b'/');
            }
            normalized.extend_from_slice(component);
        }
        Ok(Self { bytes: normalized })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn components(&self) -> impl Iterator<Item = &[u8]> {
        self.bytes.split(|byte| *byte == b'/')
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RenamePaths {
    from: RelativePath,
    to: RelativePath,
}

impl RenamePaths {
    pub fn new(from: RelativePath, to: RelativePath) -> Result<Self, DomainError> {
        if from == to {
            return Err(DomainError::InvalidRenamePaths {
                from: String::from_utf8_lossy(from.as_bytes()).into_owned(),
                to: String::from_utf8_lossy(to.as_bytes()).into_owned(),
            });
        }
        Ok(Self { from, to })
    }

    pub fn from(&self) -> &RelativePath {
        &self.from
    }

    pub fn to(&self) -> &RelativePath {
        &self.to
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UnsupportedFilesystemName(String);

impl UnsupportedFilesystemName {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        validate_text(&value, "unsupported filesystem name")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum FilesystemClass {
    Linux,
    MacOsApfs,
    Other(UnsupportedFilesystemName),
}

impl FilesystemClass {
    pub fn other(value: impl Into<String>) -> Result<Self, DomainError> {
        Ok(Self::Other(UnsupportedFilesystemName::new(value)?))
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FilesystemIdentity {
    class: FilesystemClass,
    device: u64,
    mount_id: u64,
}

impl FilesystemIdentity {
    pub const fn new(class: FilesystemClass, device: u64, mount_id: u64) -> Self {
        Self {
            class,
            device,
            mount_id,
        }
    }

    pub fn class(&self) -> &FilesystemClass {
        &self.class
    }

    pub fn device(&self) -> u64 {
        self.device
    }

    pub fn mount_id(&self) -> u64 {
        self.mount_id
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObjectIdentity {
    device: u64,
    inode: u64,
}

impl ObjectIdentity {
    pub const fn new(device: u64, inode: u64) -> Self {
        Self { device, inode }
    }

    pub fn device(&self) -> u64 {
        self.device
    }

    pub fn inode(&self) -> u64 {
        self.inode
    }
}

/// The collision and containment identity for an authority root.
///
/// A lexical path is only a display value.  The filesystem and root-object
/// identities are the authority-bearing values used for equality and scope.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AuthorityIdentity {
    filesystem: FilesystemIdentity,
    object: ObjectIdentity,
}

impl AuthorityIdentity {
    pub fn new(
        filesystem: FilesystemIdentity,
        object: ObjectIdentity,
    ) -> Result<Self, DomainError> {
        if filesystem.device() != object.device() {
            return Err(DomainError::AuthorityDeviceMismatch {
                filesystem_device: filesystem.device(),
                object_device: object.device(),
            });
        }
        Ok(Self { filesystem, object })
    }

    pub fn filesystem(&self) -> &FilesystemIdentity {
        &self.filesystem
    }

    pub fn object(&self) -> ObjectIdentity {
        self.object
    }
}

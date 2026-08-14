use super::PathError;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct RelativePath {
    pub(crate) components: Vec<Vec<u8>>,
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
    pub(crate) path: PathBuf,
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

//! Owner-selected delimiter syntax registry.
//!
//! Each supported format carries canonical open/close markers that
//! round-trip; lookup follows the decided case and extension rules;
//! unknown and extensionless cases fail closed per policy.  The registry
//! is pure data — it contains no configuration parser.

#![allow(dead_code)]

use std::{error::Error, fmt};

/// One canonical delimiter syntax.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DelimiterSyntax {
    pub format: &'static str,
    pub open: &'static str,
    pub close: &'static str,
}

impl DelimiterSyntax {
    /// The canonical marker pair in the format's own syntax.
    pub fn round_trip(&self) -> (String, String) {
        (self.open.to_owned(), self.close.to_owned())
    }
}

/// Registry failures.
#[derive(Debug)]
pub enum DelimiterError {
    /// The format is not in the registry; the policy fails closed.
    UnknownFormat { format: String },
    /// The format is extensionless and no decided rule applies.
    UnknownExtension { path: String },
    /// The format string is empty.
    Empty,
}

impl fmt::Display for DelimiterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFormat { format } => {
                write!(
                    formatter,
                    "no delimiter syntax is registered for {format:?}"
                )
            }
            Self::UnknownExtension { path } => {
                write!(formatter, "no decided delimiter rule applies to {path:?}")
            }
            Self::Empty => write!(formatter, "the format is empty"),
        }
    }
}
impl Error for DelimiterError {}

/// The decided registry: one canonical pair per supported format.
/// Lookup is exact on the format name (case-sensitive by rule).
pub const REGISTRY: &[DelimiterSyntax] = &[
    DelimiterSyntax {
        format: "yaml",
        open: "# omnirepo-start",
        close: "# omnirepo-end",
    },
    DelimiterSyntax {
        format: "toml",
        open: "# omnirepo-start",
        close: "# omnirepo-end",
    },
    DelimiterSyntax {
        format: "shell",
        open: "# omnirepo-start",
        close: "# omnirepo-end",
    },
    DelimiterSyntax {
        format: "json",
        open: "// omnirepo-start",
        close: "// omnirepo-end",
    },
    DelimiterSyntax {
        format: "javascript",
        open: "// omnirepo-start",
        close: "// omnirepo-end",
    },
    DelimiterSyntax {
        format: "typescript",
        open: "// omnirepo-start",
        close: "// omnirepo-end",
    },
    DelimiterSyntax {
        format: "markdown",
        open: "<!-- omnirepo-start -->",
        close: "<!-- omnirepo-end -->",
    },
    DelimiterSyntax {
        format: "html",
        open: "<!-- omnirepo-start -->",
        close: "<!-- omnirepo-end -->",
    },
    DelimiterSyntax {
        format: "python",
        open: "# omnirepo-start",
        close: "# omnirepo-end",
    },
    DelimiterSyntax {
        format: "rust",
        open: "// omnirepo-start",
        close: "// omnirepo-end",
    },
];

/// Look up the delimiter syntax by exact format name.  Unknown formats
/// fail closed.
pub fn lookup(format: &str) -> Result<&'static DelimiterSyntax, DelimiterError> {
    if format.is_empty() {
        return Err(DelimiterError::Empty);
    }
    REGISTRY
        .iter()
        .find(|entry| entry.format == format)
        .ok_or_else(|| DelimiterError::UnknownFormat {
            format: format.to_owned(),
        })
}

/// Resolve a file path's format by its extension (decided rule: the last
/// dot-separated component, lowercase, exact registry match).  An
/// extensionless or unknown path fails closed.
pub fn lookup_by_extension(path: &str) -> Result<&'static DelimiterSyntax, DelimiterError> {
    if path.is_empty() {
        return Err(DelimiterError::Empty);
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    let Some((_, extension)) = name.rsplit_once('.') else {
        return Err(DelimiterError::UnknownExtension {
            path: path.to_owned(),
        });
    };
    let extension = extension.to_ascii_lowercase();
    match extension.as_str() {
        "yml" | "yaml" => lookup("yaml"),
        "toml" => lookup("toml"),
        "sh" | "bash" => lookup("shell"),
        "json" => lookup("json"),
        "js" | "mjs" | "cjs" => lookup("javascript"),
        "ts" | "mts" | "cts" => lookup("typescript"),
        "md" | "markdown" => lookup("markdown"),
        "html" | "htm" => lookup("html"),
        "py" => lookup("python"),
        "rs" => lookup("rust"),
        _ => Err(DelimiterError::UnknownExtension {
            path: path.to_owned(),
        }),
    }
}

#[cfg(test)]
mod delimiters_tests;

//! Named managed-section delimiter syntax registry.
//!
//! Partial sections use exact full-line named delimiters
//! (canon/architecture/managed-content.md):
//!
//! `<comment-token> omnirepo:start <section-id>`
//! `<comment-token> omnirepo:end <section-id>`
//!
//! Each supported format carries a canonical line prefix (and, for
//! comment syntaxes that require it, a closing suffix) that round-trips;
//! lookup follows the decided case and extension rules; unknown and
//! extensionless cases fail closed per policy.  Marker recognition is
//! exact: a line that resembles a marker but is not an exact named marker
//! line is invalid, never content and never a marker.  The registry is
//! pure data — it contains no configuration parser.

#![allow(dead_code)]

use crate::configuration::SectionId;
use std::{error::Error, fmt};

/// The exact marker keywords.
pub const OPEN_KEYWORD: &str = "omnirepo:start";
pub const CLOSE_KEYWORD: &str = "omnirepo:end";

/// The previous release's unnamed marker keywords.  They are refused,
/// never treated as content: silently appending a named section beside a
/// stale legacy block would duplicate managed content without any error
/// (docs/breaking-guidance.md carries the migration).
const LEGACY_KEYWORDS: [&str; 2] = ["omnirepo-start", "omnirepo-end"];

/// One canonical delimiter syntax.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DelimiterSyntax {
    pub format: &'static str,
    /// The comment opener including its trailing space, e.g. `"# "`.
    prefix: &'static str,
    /// The comment closer including its leading space, e.g. `" -->"`;
    /// empty for line comments.
    suffix: &'static str,
}

/// The classification of one logical line (terminator excluded).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LineClass {
    /// Ordinary content: carries no marker keyword.
    Content,
    /// An exact named open marker.
    Open(SectionId),
    /// An exact named close marker.
    Close(SectionId),
    /// Carries a marker keyword but is not an exact named marker line
    /// (unnamed, whitespace-altered, invalid ID, or payload-like).
    MarkerLike { reason: String },
}

impl DelimiterSyntax {
    /// The exact open marker line for one section ID (no terminator).
    pub fn open_marker(&self, id: &SectionId) -> String {
        format!(
            "{}{OPEN_KEYWORD} {}{}",
            self.prefix,
            id.as_str(),
            self.suffix
        )
    }

    /// The exact close marker line for one section ID (no terminator).
    pub fn close_marker(&self, id: &SectionId) -> String {
        format!(
            "{}{CLOSE_KEYWORD} {}{}",
            self.prefix,
            id.as_str(),
            self.suffix
        )
    }

    /// Classify one logical line (line terminator already removed).
    ///
    /// Only a marker-shaped line must parse exactly: one that (after
    /// leading whitespace) starts with the format's comment token or with
    /// a marker keyword.  Prose that merely mentions a keyword mid-line
    /// is ordinary content — the exact-marker rule governs marker-shaped
    /// lines, not documentation about them.
    pub fn classify_line(&self, line: &[u8]) -> LineClass {
        let stripped = strip_leading_whitespace(line);
        let token = self.prefix.trim_end().as_bytes();
        let comment_shaped = stripped.starts_with(token);
        let has_open = contains(line, OPEN_KEYWORD.as_bytes());
        let has_close = contains(line, CLOSE_KEYWORD.as_bytes());
        if !has_open && !has_close {
            // Legacy unnamed markers are refused, never content.
            for legacy in LEGACY_KEYWORDS {
                if (comment_shaped || stripped.starts_with(legacy.as_bytes()))
                    && contains(line, legacy.as_bytes())
                {
                    return LineClass::MarkerLike {
                        reason: format!(
                            "the line carries the legacy unnamed marker {legacy:?}; migrate it to the named `{OPEN_KEYWORD} <section-id>` form or remove it"
                        ),
                    };
                }
            }
            return LineClass::Content;
        }
        let keyword_shaped = stripped.starts_with(OPEN_KEYWORD.as_bytes())
            || stripped.starts_with(CLOSE_KEYWORD.as_bytes());
        if !comment_shaped && !keyword_shaped {
            return LineClass::Content;
        }
        // omnirepo:start contains no omnirepo:end and vice versa, so the
        // keyword present decides which exact form the line must take.
        let keyword = if has_open {
            OPEN_KEYWORD
        } else {
            CLOSE_KEYWORD
        };
        match self.parse_marker(line, keyword) {
            Ok(id) if has_open => LineClass::Open(id),
            Ok(id) => LineClass::Close(id),
            Err(reason) => LineClass::MarkerLike { reason },
        }
    }

    /// Parse one exact named marker line: prefix, keyword, one space, a
    /// valid section ID, suffix — nothing more, nothing less.
    fn parse_marker(&self, line: &[u8], keyword: &str) -> Result<SectionId, String> {
        let Some(rest) = line.strip_prefix(self.prefix.as_bytes()) else {
            return Err(format!(
                "the marker line must start with {:?} exactly",
                self.prefix
            ));
        };
        let Some(rest) = rest.strip_prefix(keyword.as_bytes()) else {
            return Err(format!("the marker keyword must be {keyword:?} exactly"));
        };
        let Some(rest) = rest.strip_prefix(b" ") else {
            return Err("the marker keyword must be followed by one space and a section id".into());
        };
        let Some(id_bytes) = rest.strip_suffix(self.suffix.as_bytes()) else {
            return Err(format!(
                "the marker line must end with {:?} exactly",
                self.suffix
            ));
        };
        let Ok(id) = std::str::from_utf8(id_bytes) else {
            return Err("the section id is not valid UTF-8".into());
        };
        // SectionId::new owns the ID rule and its error wording.
        SectionId::new(id).map_err(|error| error.to_string())
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn strip_leading_whitespace(line: &[u8]) -> &[u8] {
    let start = line
        .iter()
        .position(|byte| *byte != b' ' && *byte != b'\t')
        .unwrap_or(line.len());
    &line[start..]
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

/// The decided registry: one canonical syntax per supported format.
/// Lookup is exact on the format name (case-sensitive by rule).
pub const REGISTRY: &[DelimiterSyntax] = &[
    DelimiterSyntax {
        format: "yaml",
        prefix: "# ",
        suffix: "",
    },
    DelimiterSyntax {
        format: "toml",
        prefix: "# ",
        suffix: "",
    },
    DelimiterSyntax {
        format: "shell",
        prefix: "# ",
        suffix: "",
    },
    DelimiterSyntax {
        format: "python",
        prefix: "# ",
        suffix: "",
    },
    DelimiterSyntax {
        format: "ruby",
        prefix: "# ",
        suffix: "",
    },
    DelimiterSyntax {
        format: "ini",
        prefix: "; ",
        suffix: "",
    },
    DelimiterSyntax {
        format: "sql",
        prefix: "-- ",
        suffix: "",
    },
    DelimiterSyntax {
        format: "json",
        prefix: "// ",
        suffix: "",
    },
    DelimiterSyntax {
        format: "javascript",
        prefix: "// ",
        suffix: "",
    },
    DelimiterSyntax {
        format: "typescript",
        prefix: "// ",
        suffix: "",
    },
    DelimiterSyntax {
        format: "rust",
        prefix: "// ",
        suffix: "",
    },
    DelimiterSyntax {
        format: "markdown",
        prefix: "<!-- ",
        suffix: " -->",
    },
    DelimiterSyntax {
        format: "html",
        prefix: "<!-- ",
        suffix: " -->",
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
        "py" => lookup("python"),
        "rb" => lookup("ruby"),
        "ini" => lookup("ini"),
        "sql" => lookup("sql"),
        "json" => lookup("json"),
        "js" | "mjs" | "cjs" => lookup("javascript"),
        "ts" | "mts" | "cts" => lookup("typescript"),
        "rs" => lookup("rust"),
        "md" | "markdown" => lookup("markdown"),
        "html" | "htm" => lookup("html"),
        _ => Err(DelimiterError::UnknownExtension {
            path: path.to_owned(),
        }),
    }
}

#[cfg(test)]
mod delimiters_tests;

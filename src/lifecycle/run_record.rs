//! Durable creation of the first record for a fleet run.
//!
//! This module owns only the invocation-boundary effect.  The caller must
//! create a record before it reads machine or source authority, admits a
//! repository, or starts any other effect.  Later journal events belong to the
//! persistence writer and are intentionally not part of this seam.

use crate::platform::{MutationIntent, PathError, RelativePath, RunRecordRoot, open_mutation_root};
use std::{
    error::Error,
    fmt,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(test)]
mod run_record_tests;

const JOURNAL_VERSION: u8 = 1;
const JOURNAL_MODE: u32 = 0o600;
const RUN_SUFFIX_BYTES: usize = 16;
const RUN_DIRECTORY: &str = ".omnirepo/runs";

/// The stable version of the first JSONL record.
#[allow(dead_code)]
pub const RECORD_VERSION: u8 = JOURNAL_VERSION;

/// A timestamp plus a 128-bit random suffix.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RunId {
    timestamp: String,
    suffix: String,
}

impl RunId {
    fn from_parts(
        timestamp: SystemTime,
        suffix: [u8; RUN_SUFFIX_BYTES],
    ) -> Result<Self, RunRecordError> {
        let timestamp = format_utc_timestamp(timestamp)?;
        let suffix = hex(&suffix);
        Ok(Self { timestamp, suffix })
    }

    /// The UTC component of the identifier.
    pub fn timestamp(&self) -> &str {
        &self.timestamp
    }

    /// The lowercase hexadecimal 128-bit suffix.
    pub fn suffix(&self) -> &str {
        &self.suffix
    }

    /// The filename stem used below the run-record directory.
    pub fn file_name(&self) -> String {
        format!("{}-{}.log", self.timestamp, self.suffix)
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}-{}", self.timestamp, self.suffix)
    }
}

/// Failure while creating the invocation-boundary record.
///
/// Every variant is a pre-record failure.  The caller must not continue to a
/// fleet or repository effect after receiving one.
#[derive(Debug)]
pub enum RunRecordError {
    InvalidHome { path: PathBuf, reason: &'static str },
    ParentUnavailable { path: PathBuf, reason: String },
    ParentRejected { path: PathBuf, reason: String },
    Collision { path: PathBuf },
    Create { path: PathBuf, reason: String },
    Permission { path: PathBuf, reason: String },
    Write { path: PathBuf, reason: String },
    Clock { reason: String },
    Entropy { reason: String },
}

impl fmt::Display for RunRecordError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHome { path, reason } => {
                write!(formatter, "invalid HOME {:?}: {reason}", path)
            }
            Self::ParentUnavailable { path, reason } => {
                write!(
                    formatter,
                    "run-record parent unavailable {:?}: {reason}",
                    path
                )
            }
            Self::ParentRejected { path, reason } => {
                write!(formatter, "run-record parent rejected {:?}: {reason}", path)
            }
            Self::Collision { path } => {
                write!(formatter, "run-record path already exists: {:?}", path)
            }
            Self::Create { path, reason } => {
                write!(
                    formatter,
                    "run-record creation failed for {:?}: {reason}",
                    path
                )
            }
            Self::Permission { path, reason } => {
                write!(
                    formatter,
                    "run-record permissions failed for {:?}: {reason}",
                    path
                )
            }
            Self::Write { path, reason } => {
                write!(
                    formatter,
                    "run-record write failed for {:?}: {reason}",
                    path
                )
            }
            Self::Clock { reason } => write!(formatter, "run-record clock failed: {reason}"),
            Self::Entropy { reason } => write!(formatter, "run-record entropy failed: {reason}"),
        }
    }
}

impl Error for RunRecordError {}

/// The exclusively created, mode-0600 journal and its canonical identity.
#[derive(Debug)]
pub struct RunRecord {
    id: RunId,
    path: PathBuf,
    file: File,
}

impl RunRecord {
    /// Create a run record using the operating system clock and entropy source.
    pub fn create(home: impl AsRef<Path>) -> Result<Self, RunRecordError> {
        let timestamp = SystemTime::now();
        let mut suffix = [0_u8; RUN_SUFFIX_BYTES];
        fill_os_entropy(&mut suffix)?;
        Self::create_with_id(home, timestamp, suffix)
    }

    /// Create a record with injected identity inputs.
    ///
    /// The deterministic seam is used by tests and recovery proofs.  It does
    /// not weaken exclusive creation: an already occupied identity is always
    /// a collision and is never overwritten.
    pub fn create_with_id(
        home: impl AsRef<Path>,
        timestamp: SystemTime,
        suffix: [u8; RUN_SUFFIX_BYTES],
    ) -> Result<Self, RunRecordError> {
        let home = home.as_ref();
        validate_home(home)?;
        let runs_directory = home.join(RUN_DIRECTORY);
        let id = RunId::from_parts(timestamp, suffix)?;
        Self::create_in_directory(&runs_directory, id)
    }

    /// Create a record in an already selected, canonical run directory.
    ///
    /// This is `pub(crate)` so the future composition root can keep HOME
    /// discovery separate from the file operation while the public command
    /// remains binary-only.
    pub(crate) fn create_in_directory(
        runs_directory: &Path,
        id: RunId,
    ) -> Result<Self, RunRecordError> {
        validate_absolute_directory_path(runs_directory)?;
        let file_name = id.file_name();
        let relative = RelativePath::parse(&file_name).map_err(|error| RunRecordError::Create {
            path: runs_directory.join(&file_name),
            reason: error.to_string(),
        })?;
        let root = open_mutation_root::<RunRecordRoot>(runs_directory)
            .map_err(|error| map_parent_error(runs_directory, error))?;
        let target = root
            .resolve_mutation(&relative, MutationIntent::CreateExclusive)
            .map_err(|error| map_create_error(&runs_directory.join(&file_name), error))?;
        let directory = target
            .clone_parent()
            .map_err(|error| map_parent_error(runs_directory, error))?;
        let mut file = target
            .create_exclusive_with_mode(JOURNAL_MODE)
            .map_err(|error| map_create_error(&runs_directory.join(&file_name), error))?;
        let line = initial_intent_line(&id);
        if let Err(error) = file.write_all(line.as_bytes()) {
            return Err(RunRecordError::Write {
                path: runs_directory.join(&file_name),
                reason: error.to_string(),
            });
        }
        crate::platform::sync_file(
            &file,
            &runs_directory.join(&file_name).display().to_string(),
        )
        .map_err(|error| RunRecordError::Write {
            path: runs_directory.join(&file_name),
            reason: format!("sync initial intent: {error}"),
        })?;
        crate::platform::sync_directory(&directory, &runs_directory.display().to_string())
            .map_err(|error| RunRecordError::Write {
                path: runs_directory.to_path_buf(),
                reason: format!("sync run-record directory: {error}"),
            })?;

        Ok(Self {
            id,
            path: runs_directory.join(file_name),
            file,
        })
    }

    /// The stable run identity.
    pub fn id(&self) -> &RunId {
        &self.id
    }

    /// The path returned to the caller after successful creation.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file remains private so later journal stages cannot bypass the
    /// record writer's ordering and evidence policy.
    #[allow(dead_code)]
    pub(crate) fn file_mut(&mut self) -> &mut File {
        &mut self.file
    }
}

fn validate_home(home: &Path) -> Result<(), RunRecordError> {
    if !home.is_absolute() {
        return Err(RunRecordError::InvalidHome {
            path: home.to_path_buf(),
            reason: "HOME must be absolute",
        });
    }
    validate_absolute_directory_path(home)
}

fn validate_absolute_directory_path(path: &Path) -> Result<(), RunRecordError> {
    if !path.is_absolute() {
        return Err(RunRecordError::InvalidHome {
            path: path.to_path_buf(),
            reason: "authority path must be absolute",
        });
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(RunRecordError::InvalidHome {
            path: path.to_path_buf(),
            reason: "authority path cannot contain parent traversal",
        });
    }
    Ok(())
}

fn map_parent_error(path: &Path, error: PathError) -> RunRecordError {
    match error {
        PathError::NotFound { path: detail } => RunRecordError::ParentUnavailable {
            path: path.to_path_buf(),
            reason: detail,
        },
        PathError::LinkLikeObject { path: detail }
        | PathError::MountCrossing { path: detail }
        | PathError::UnsupportedObject { path: detail, .. }
        | PathError::InvalidAuthorityRoot { path: detail, .. }
        | PathError::UnsupportedFilesystem { path: detail, .. } => RunRecordError::ParentRejected {
            path: path.to_path_buf(),
            reason: detail,
        },
        other => RunRecordError::Create {
            path: path.to_path_buf(),
            reason: other.to_string(),
        },
    }
}

fn map_create_error(path: &Path, error: PathError) -> RunRecordError {
    match error {
        PathError::Io { code: Some(17), .. } => RunRecordError::Collision {
            path: path.to_path_buf(),
        },
        PathError::LinkLikeObject { .. }
        | PathError::MountCrossing { .. }
        | PathError::UnsupportedObject { .. }
        | PathError::UnsafeHardLink { .. } => RunRecordError::ParentRejected {
            path: path.to_path_buf(),
            reason: error.to_string(),
        },
        PathError::Io {
            code: Some(13 | 1), ..
        } => RunRecordError::Permission {
            path: path.to_path_buf(),
            reason: error.to_string(),
        },
        other => RunRecordError::Create {
            path: path.to_path_buf(),
            reason: other.to_string(),
        },
    }
}

fn initial_intent_line(id: &RunId) -> String {
    format!(
        "{{\"version\":{JOURNAL_VERSION},\"type\":\"run_intent\",\"run_id\":\"{}\",\"created_at\":\"{}\",\"stage\":\"invocation\",\"status\":\"started\"}}\n",
        id,
        id.timestamp()
    )
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

fn format_utc_timestamp(timestamp: SystemTime) -> Result<String, RunRecordError> {
    let duration = timestamp
        .duration_since(UNIX_EPOCH)
        .map_err(|error| RunRecordError::Clock {
            reason: error.to_string(),
        })?;
    let seconds = duration.as_secs();
    let days = seconds / 86_400;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days).ok_or_else(|| RunRecordError::Clock {
        reason: "timestamp is outside the supported calendar range".to_owned(),
    })?;
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    Ok(format!(
        "{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}Z"
    ))
}

// Howard Hinnant's proleptic Gregorian conversion, expressed without a
// dependency so the first-run path adds no runtime package surface.
fn civil_from_days(days_since_epoch: u64) -> Option<(i64, u64, u64)> {
    let days = i64::try_from(days_since_epoch).ok()?;
    let days = days.checked_add(719_468)?;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };
    Some((year, u64::try_from(month).ok()?, u64::try_from(day).ok()?))
}

fn fill_os_entropy(bytes: &mut [u8; RUN_SUFFIX_BYTES]) -> Result<(), RunRecordError> {
    #[cfg(unix)]
    {
        let mut source = File::open("/dev/urandom").map_err(|error| RunRecordError::Entropy {
            reason: error.to_string(),
        })?;
        source
            .read_exact(bytes)
            .map_err(|error| RunRecordError::Entropy {
                reason: error.to_string(),
            })?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = bytes;
        Err(RunRecordError::Entropy {
            reason: "the supported platform set has no operating-system entropy source".to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn timestamp_format_is_utc_and_stable() {
        let timestamp = UNIX_EPOCH + Duration::from_secs(1_754_064_000);
        assert_eq!(
            format_utc_timestamp(timestamp).expect("timestamp formats"),
            "20250801T160000Z"
        );
    }

    #[test]
    fn suffix_is_lowercase_hex() {
        assert_eq!(hex(&[0x00, 0x01, 0xab, 0xff]), "0001abff");
    }
}

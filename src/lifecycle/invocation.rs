//! Invocation-to-run sequencing at the owner-decided boundary (.24/.27).
//!
//! A syntactically valid `sync` invocation becomes a fleet run immediately
//! after CLI parsing and before machine configuration, source acquisition,
//! or any destination effect: the durable record is created first, the
//! journal writer starts over it, exactly one canonical initial application
//! service is dispatched, and the run is finalized with the dispatch
//! outcome.  A record-creation failure produces zero effects and exits with
//! the durable-record class (5).  Help, version, parse errors, `setup`, and
//! `validate` are never fleet runs.

#![allow(dead_code)]

use super::journal::{Journal, JournalConfig};
use super::run_record::{RunRecord, RunRecordError};
use crate::configuration::Command;
use std::{error::Error, fmt, path::PathBuf};

/// Canonical application dispatch failure.
#[derive(Debug)]
pub(crate) enum ApplicationError {
    /// The canonical initial application service has not landed yet.
    Unavailable,
}

impl fmt::Display for ApplicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter
                .write_str("the canonical application service is not available in this build"),
        }
    }
}
impl Error for ApplicationError {}

/// Run one parsed invocation and return the owner exit contract code.
pub(crate) fn run_invocation(command: Command) -> u8 {
    match command {
        Command::Sync => run_sync(),
        Command::Setup(_) => unavailable("setup"),
        Command::Doctor => {
            let Some(home) = canonical_home() else {
                eprintln!("omnirepo: doctor failed: HOME is not an absolute directory");
                return 2;
            };
            // Doctor is never a fleet run: it reports and creates no
            // record.  The report is the command's product output.
            let report = super::doctor::diagnose(&home);
            print!("{}", report.render());
            if report.healthy() {
                println!("doctor: healthy");
                0
            } else {
                println!("doctor: problems found");
                2
            }
        }
    }
}

fn unavailable(command: &str) -> u8 {
    eprintln!(
        "omnirepo: {command} is not available in this build; the constitutional lifecycle lands in a later delivery slice"
    );
    2
}

fn run_sync() -> u8 {
    let Some(home) = canonical_home() else {
        eprintln!(
            "omnirepo: sync failed: HOME is not an absolute directory; the run record cannot be created"
        );
        return 2;
    };
    if let Err(error) = RunRecord::ensure_runs_directory(&home) {
        eprintln!("omnirepo: sync failed: cannot establish the run-record directory: {error}");
        return 5;
    }
    let record = match RunRecord::create(&home) {
        Ok(record) => record,
        Err(RunRecordError::InvalidHome { path, reason }) => {
            eprintln!(
                "omnirepo: sync failed: invalid run-record home {}: {reason}",
                path.display()
            );
            return 2;
        }
        Err(error) => {
            eprintln!("omnirepo: sync failed: cannot create the run record: {error}");
            return 5;
        }
    };
    let run_id = record.id().to_string();
    let record_path = record.path().to_path_buf();
    let mut journal = Journal::start(record, JournalConfig::default());
    // Dispatch exactly one canonical initial application service: an
    // absent machine authority is an empty fleet, which is a success
    // (the .27 contract); a configured fleet composes and runs the
    // canonical pipeline.  The dispatch already wrote its terminal
    // event; this boundary only maps the exit class.
    let outcome =
        super::fleet_dispatch::dispatch_fleet(&journal.handle, &run_id, &home, &record_path);
    let exit_code = match &outcome {
        Ok(dispatch) => super::exit_status::exit_code_for(dispatch.exit_class) as u8,
        Err(_) => 2,
    };
    if let Err(error) = journal.shutdown() {
        eprintln!("omnirepo: sync failed: cannot finalize the run journal: {error}");
        return 5;
    }
    match outcome {
        Ok(dispatch) => {
            if dispatch.exit_class == super::exit_status::ExitClass::Success {
                eprintln!(
                    "omnirepo: sync completed (run {run_id} recorded at {})",
                    record_path.display()
                );
            } else {
                eprintln!(
                    "omnirepo: sync failed (run {run_id} recorded at {})",
                    record_path.display()
                );
            }
            exit_code
        }
        Err(error) => {
            eprintln!(
                "omnirepo: sync failed: {error} (run {run_id} recorded at {})",
                record_path.display()
            );
            exit_code
        }
    }
}

/// Resolve the canonical configuration home without ambient scanning.
fn canonical_home() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let path = PathBuf::from(home);
    if path.is_absolute() && path.is_dir() {
        Some(path)
    } else {
        None
    }
}

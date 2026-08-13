//! Reusable, deterministic scheduler-configuration boundary fixture.
//!
//! This is a small test model for the configuration and admission contract.
//! It is not a scheduler implementation. Consumers can include this module
//! with `#[path = ".../concurrency_fixture_api.rs"]` while building the
//! production scheduler.

const CASE_DATA: &str = include_str!("concurrency_boundary_cases.tsv");

pub const DEFAULT_MAX_REPOSITORIES: u16 = 4;
pub const DEFAULT_MAX_CHILD_WORK: u16 = 8;
pub const MAX_REPOSITORIES: u16 = 32;
pub const MAX_CHILD_WORK: u16 = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryCase {
    pub name: String,
    pub machine_repositories: String,
    pub machine_child_work: String,
    pub repository_override: String,
    pub child_override: String,
    pub work_items: usize,
    pub cancellation: Cancellation,
    pub expected: Expected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cancellation {
    None,
    Queued,
    Active,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Expected {
    Parsed {
        max_repositories: u16,
        max_child_work: u16,
        trace: Vec<String>,
    },
    Error {
        field: String,
        kind: ErrorKind,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Zero,
    Negative,
    Fractional,
    NonInteger,
    Null,
    OutOfRange,
    UnknownField,
    DuplicateField,
    OverrideAboveMachine,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoundaryError {
    pub field: &'static str,
    pub kind: ErrorKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectiveConfig {
    pub max_repositories: u16,
    pub max_child_work: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Evaluation {
    pub parsed: EffectiveConfig,
    pub trace: Vec<String>,
}

/// Parse all table rows. The fixture panics if its own schema is malformed.
pub fn cases() -> Vec<BoundaryCase> {
    CASE_DATA
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .map(|(line_number, line)| parse_case(line_number + 1, line))
        .collect()
}

pub fn case(name: &str) -> Option<BoundaryCase> {
    cases().into_iter().find(|candidate| candidate.name == name)
}

/// Parse a row with the same defaults, bounds, override rule, and deterministic
/// admission events that the production scheduler must expose.
pub fn parse_and_trace(case: &BoundaryCase) -> Result<Evaluation, BoundaryError> {
    let machine_repositories = parse_limit(
        "max_repositories",
        &case.machine_repositories,
        DEFAULT_MAX_REPOSITORIES,
        MAX_REPOSITORIES,
    )?;
    let machine_child_work = parse_limit(
        "max_child_work",
        &case.machine_child_work,
        DEFAULT_MAX_CHILD_WORK,
        MAX_CHILD_WORK,
    )?;

    let repository_override = parse_override(
        "max_repositories",
        &case.repository_override,
        MAX_REPOSITORIES,
    )?;
    let child_override = parse_override("max_child_work", &case.child_override, MAX_CHILD_WORK)?;

    if repository_override.is_some_and(|value| value > machine_repositories) {
        return Err(BoundaryError {
            field: "max_repositories",
            kind: ErrorKind::OverrideAboveMachine,
        });
    }
    if child_override.is_some_and(|value| value > machine_child_work) {
        return Err(BoundaryError {
            field: "max_child_work",
            kind: ErrorKind::OverrideAboveMachine,
        });
    }

    let config = EffectiveConfig {
        max_repositories: repository_override.unwrap_or(machine_repositories),
        max_child_work: child_override.unwrap_or(machine_child_work),
    };

    Ok(Evaluation {
        parsed: config.clone(),
        trace: admission_trace(&config, case.work_items, case.cancellation),
    })
}

fn parse_case(line_number: usize, line: &str) -> BoundaryCase {
    let columns: Vec<&str> = line.split('\t').collect();
    assert_eq!(
        columns.len(),
        11,
        "fixture line {line_number} must have 11 tab-separated columns"
    );

    let work_items = columns[5].parse::<usize>().unwrap_or_else(|_| {
        panic!(
            "fixture line {line_number} has invalid work item count {:?}",
            columns[5]
        )
    });
    let cancellation = match columns[6] {
        "none" => Cancellation::None,
        "queued" => Cancellation::Queued,
        "active" => Cancellation::Active,
        value => panic!("fixture line {line_number} has invalid cancellation {value:?}"),
    };

    let expected = match columns[7] {
        "ok" => Expected::Parsed {
            max_repositories: parse_expected_u16(line_number, columns[8]),
            max_child_work: parse_expected_u16(line_number, columns[9]),
            trace: columns[10].split('>').map(str::to_owned).collect(),
        },
        "error" => {
            let (field, kind) = parse_expected_error(line_number, columns[10]);
            Expected::Error { field, kind }
        }
        value => panic!("fixture line {line_number} has invalid result {value:?}"),
    };

    BoundaryCase {
        name: columns[0].to_owned(),
        machine_repositories: columns[1].to_owned(),
        machine_child_work: columns[2].to_owned(),
        repository_override: columns[3].to_owned(),
        child_override: columns[4].to_owned(),
        work_items,
        cancellation,
        expected,
    }
}

fn parse_expected_u16(line_number: usize, value: &str) -> u16 {
    value.parse::<u16>().unwrap_or_else(|_| {
        panic!("fixture line {line_number} has invalid expected number {value:?}")
    })
}

fn parse_expected_error(line_number: usize, value: &str) -> (String, ErrorKind) {
    let (field, kind) = value
        .split_once(':')
        .unwrap_or_else(|| panic!("fixture line {line_number} error must use field:kind syntax"));
    let kind = match kind {
        "zero" => ErrorKind::Zero,
        "negative" => ErrorKind::Negative,
        "fractional" => ErrorKind::Fractional,
        "non_integer" => ErrorKind::NonInteger,
        "null" => ErrorKind::Null,
        "out_of_range" => ErrorKind::OutOfRange,
        "unknown_field" => ErrorKind::UnknownField,
        "duplicate_field" => ErrorKind::DuplicateField,
        "override_above_machine" => ErrorKind::OverrideAboveMachine,
        value => panic!("fixture line {line_number} has invalid error kind {value:?}"),
    };
    (field.to_owned(), kind)
}

fn parse_limit(
    field: &'static str,
    raw: &str,
    default: u16,
    maximum: u16,
) -> Result<u16, BoundaryError> {
    if raw == "-" {
        return Ok(default);
    }

    parse_value(field, raw, maximum)
}

fn parse_override(
    field: &'static str,
    raw: &str,
    maximum: u16,
) -> Result<Option<u16>, BoundaryError> {
    if raw == "-" {
        return Ok(None);
    }

    parse_value(field, raw, maximum).map(Some)
}

fn parse_value(field: &'static str, raw: &str, maximum: u16) -> Result<u16, BoundaryError> {
    let kind = if raw == "unknown" {
        Some(ErrorKind::UnknownField)
    } else if raw == "duplicate" {
        Some(ErrorKind::DuplicateField)
    } else if raw == "null" {
        Some(ErrorKind::Null)
    } else if raw.starts_with('-') {
        Some(ErrorKind::Negative)
    } else if raw.contains('.') {
        Some(ErrorKind::Fractional)
    } else {
        None
    };

    if let Some(kind) = kind {
        return Err(BoundaryError { field, kind });
    }

    let value = raw.parse::<u128>().map_err(|_| BoundaryError {
        field,
        kind: ErrorKind::NonInteger,
    })?;
    if value == 0 {
        return Err(BoundaryError {
            field,
            kind: ErrorKind::Zero,
        });
    }
    if value > u128::from(maximum) {
        return Err(BoundaryError {
            field,
            kind: ErrorKind::OutOfRange,
        });
    }

    Ok(value as u16)
}

fn admission_trace(
    config: &EffectiveConfig,
    work_items: usize,
    cancellation: Cancellation,
) -> Vec<String> {
    let mut trace = Vec::new();

    for ticket in 1..=work_items {
        trace.push(format!("repository.queued:{ticket}"));
    }

    let admitted = work_items.min(usize::from(config.max_repositories));
    for ticket in 1..=admitted {
        trace.push(format!("repository.admitted:{ticket}"));
    }

    for ticket in 1..=admitted {
        trace.push(format!("child.queued:{ticket}"));
    }

    match cancellation {
        Cancellation::None => {
            let mut remaining = admitted;
            while remaining > 0 {
                let batch = remaining.min(usize::from(config.max_child_work));
                trace.push(format!("child.admitted:{batch}"));
                trace.push("child.released:0".to_owned());
                remaining -= batch;
            }
            for remaining in (0..admitted).rev() {
                trace.push(format!("repository.released:{remaining}"));
            }
        }
        Cancellation::Queued => {
            let mut remaining = admitted;
            while remaining > 0 {
                let batch = remaining.min(usize::from(config.max_child_work));
                trace.push(format!("child.admitted:{batch}"));
                trace.push("child.released:0".to_owned());
                remaining -= batch;
            }
            trace.push("cancellation.requested".to_owned());
            for _ in admitted..work_items {
                trace.push("repository.cancelled:queued".to_owned());
            }
            for remaining in (0..admitted).rev() {
                trace.push(format!("repository.released:{remaining}"));
            }
        }
        Cancellation::Active => {
            if admitted > 0 {
                let active_children = admitted.min(usize::from(config.max_child_work));
                trace.push(format!("child.admitted:{active_children}"));
                trace.push("cancellation.requested".to_owned());
                trace.push("child.termination_requested".to_owned());
                trace.push("child.reaped".to_owned());
                trace.push("child.released:0".to_owned());
                trace.push("repository.cancelled:active".to_owned());
                trace.push(format!("repository.released:{}", admitted - 1));
            }
            for _ in 1..admitted {
                trace.push("repository.cancelled:active".to_owned());
            }
        }
    }

    trace
}

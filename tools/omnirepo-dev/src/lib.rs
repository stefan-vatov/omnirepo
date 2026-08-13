//! Testable seams for repository-only developer tooling.
//!
//! Product behavior does not live in this crate. Beads validation is read-only
//! repository tooling and is intentionally separate from the shipped
//! `omnirepo` binary.

pub mod beads_validator;
pub mod br_adapter;
pub mod changed_coverage;
pub mod coverage;
pub mod planner;
pub mod quality;
pub mod test_suite;
pub mod transition_matrix;
pub mod viewer;

#[cfg(test)]
mod beads_validator_tests;

/// The stable help output of the private developer-tool dispatcher.
pub const HELP: &str = "omnirepo-dev: private repository tooling\n\nUsage:\n  omnirepo-dev validate-decisions [--json]\n  omnirepo-dev plan --repo-root PATH [--tracked-jsonl PATH] --json\n  omnirepo-dev quality --manifest PATH --repo-root PATH [--profile NAME] --json\n  omnirepo-dev test --manifest PATH --repo-root PATH [--case ID | --suite ID | --full] [--jobs N] [--artifacts PATH] [--quality-profile NAME] --json\n  omnirepo-dev coverage-ownership --repo-root PATH --lcov PATH --matrix PATH --ownership PATH --json\n  omnirepo-dev changed-coverage --repo-root PATH --lcov PATH [--base REVISION] [--head REVISION] [--report PATH] --json\n  omnirepo-dev transition-matrix --repo-root PATH --json\n  omnirepo-dev viewer refresh --input PATH --json\n";

/// The private tool's version, supplied by Cargo metadata.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The result of one developer-tool command invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub status: u8,
}

/// Dispatch the currently supported developer-tool commands.
pub fn run<I, S>(arguments: I) -> CommandOutput
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let arguments = arguments
        .into_iter()
        .map(|argument| argument.as_ref().to_owned())
        .collect::<Vec<_>>();

    match arguments.as_slice() {
        [] => CommandOutput {
            stdout: HELP.to_owned(),
            stderr: String::new(),
            status: 0,
        },
        [arg] if arg == "-h" || arg == "--help" => CommandOutput {
            stdout: HELP.to_owned(),
            stderr: String::new(),
            status: 0,
        },
        [arg] if arg == "-V" || arg == "--version" => CommandOutput {
            stdout: format!("omnirepo-dev {VERSION}\n"),
            stderr: String::new(),
            status: 0,
        },
        [command, rest @ ..] if command == "validate-decisions" => run_validate_decisions(rest),
        [command, rest @ ..] if command == "plan" => run_plan(rest),
        [command, rest @ ..] if command == "quality" => run_quality(rest),
        [command, rest @ ..] if command == "test" => run_test_suite(rest),
        [command, rest @ ..] if command == "coverage-ownership" => run_coverage_ownership(rest),
        [command, rest @ ..] if command == "changed-coverage" => run_changed_coverage(rest),
        [command, rest @ ..] if command == "transition-matrix" => run_transition_matrix(rest),
        [command, rest @ ..] if command == "viewer" => run_viewer(rest),
        [command, ..] => CommandOutput {
            stdout: String::new(),
            stderr: format!("omnirepo-dev: unsupported developer command: {command}\n"),
            status: 2,
        },
    }
}

fn run_plan(arguments: &[String]) -> CommandOutput {
    let mut repository_root = None;
    let mut tracked_jsonl = None;
    let mut json = false;
    let mut index = 0;

    while index < arguments.len() {
        match arguments[index].as_str() {
            "--repo-root" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return plan_usage_error("--repo-root requires a path");
                };
                repository_root = Some(path.clone());
            }
            "--tracked-jsonl" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return plan_usage_error("--tracked-jsonl requires a path");
                };
                tracked_jsonl = Some(path.clone());
            }
            "--json" => json = true,
            "--help" | "-h" => {
                return CommandOutput {
                    stdout: HELP.to_owned(),
                    stderr: String::new(),
                    status: 0,
                };
            }
            unsupported => {
                return plan_usage_error(&format!("unsupported plan argument: {unsupported}"));
            }
        }
        index += 1;
    }

    if !json {
        return plan_usage_error("plan requires --json");
    }

    let repository_root = repository_root
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        });
    let repository_root = repository_root.canonicalize().unwrap_or(repository_root);
    let tracked_jsonl = tracked_jsonl
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("BEADS_JSONL").map(std::path::PathBuf::from))
        .unwrap_or_else(|| repository_root.join(planner::DEFAULT_TRACKED_JSONL));
    let tracked_jsonl = if tracked_jsonl.is_absolute() {
        tracked_jsonl
    } else {
        repository_root.join(tracked_jsonl)
    };

    let report = match planner::discover(&repository_root, tracked_jsonl) {
        Ok(planner) => planner.run(),
        Err(error) => planner::report_for_adapter_error(error),
    };
    let status = if report.status == planner::PlanStatus::Ok {
        0
    } else {
        1
    };
    CommandOutput {
        stdout: format!("{report}\n"),
        stderr: String::new(),
        status,
    }
}

fn plan_usage_error(message: &str) -> CommandOutput {
    CommandOutput {
        stdout: String::new(),
        stderr: format!("omnirepo-dev: {message}\n"),
        status: 2,
    }
}

fn run_transition_matrix(arguments: &[String]) -> CommandOutput {
    let mut repository_root = None;
    let mut json = false;
    let mut index = 0;

    while index < arguments.len() {
        match arguments[index].as_str() {
            "--repo-root" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return transition_matrix_usage_error("--repo-root requires a path");
                };
                repository_root = Some(path.clone());
            }
            "--json" => json = true,
            "--help" | "-h" => {
                return CommandOutput {
                    stdout: HELP.to_owned(),
                    stderr: String::new(),
                    status: 0,
                };
            }
            unsupported => {
                return transition_matrix_usage_error(&format!(
                    "unsupported transition-matrix argument: {unsupported}"
                ));
            }
        }
        index += 1;
    }

    if !json {
        return transition_matrix_usage_error("transition-matrix requires --json");
    }
    let repository_root = repository_root
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        });
    match transition_matrix::run(&repository_root) {
        Ok(report) => CommandOutput {
            stdout: serde_json::to_string(&report)
                .expect("transition matrix report serialization is infallible")
                + "\n",
            stderr: String::new(),
            status: 0,
        },
        Err(error) => CommandOutput {
            stdout: String::new(),
            stderr: format!("omnirepo-dev: transition matrix failed: {error}\n"),
            status: 1,
        },
    }
}

fn transition_matrix_usage_error(message: &str) -> CommandOutput {
    CommandOutput {
        stdout: String::new(),
        stderr: format!("omnirepo-dev: {message}\n"),
        status: 2,
    }
}

fn run_quality(arguments: &[String]) -> CommandOutput {
    let mut manifest = None;
    let mut repo_root = None;
    let mut profile = None;
    let mut json = false;
    let mut index = 0;

    while index < arguments.len() {
        match arguments[index].as_str() {
            "--manifest" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return quality_usage_error("--manifest requires a path");
                };
                manifest = Some(path.clone());
            }
            "--repo-root" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return quality_usage_error("--repo-root requires a path");
                };
                repo_root = Some(path.clone());
            }
            "--profile" => {
                index += 1;
                let Some(name) = arguments.get(index) else {
                    return quality_usage_error("--profile requires a name");
                };
                profile = Some(name.clone());
            }
            "--json" => json = true,
            "--help" | "-h" => {
                return CommandOutput {
                    stdout: HELP.to_owned(),
                    stderr: String::new(),
                    status: 0,
                };
            }
            unsupported => {
                return quality_usage_error(&format!(
                    "unsupported quality argument: {unsupported}"
                ));
            }
        }
        index += 1;
    }

    if !json {
        return quality_usage_error("quality requires --json");
    }
    let Some(manifest) = manifest else {
        return quality_usage_error("quality requires --manifest PATH");
    };
    let Some(repo_root) = repo_root else {
        return quality_usage_error("quality requires --repo-root PATH");
    };

    let options = quality::RunnerOptions::new(manifest, repo_root);
    let options = match profile {
        Some(profile) => options.with_profile(profile),
        None => options,
    };
    match quality::run(&options) {
        Ok(report) => CommandOutput {
            stdout: quality::render_json(&report)
                .expect("quality report serialization is infallible")
                + "\n",
            stderr: String::new(),
            status: report.exit_code as u8,
        },
        Err(error) => CommandOutput {
            stdout: String::new(),
            stderr: format!("omnirepo-dev: {error}\n"),
            status: 2,
        },
    }
}

fn quality_usage_error(message: &str) -> CommandOutput {
    CommandOutput {
        stdout: String::new(),
        stderr: format!("omnirepo-dev: {message}\n"),
        status: 2,
    }
}

fn run_test_suite(arguments: &[String]) -> CommandOutput {
    let mut manifest = None;
    let mut repo_root = None;
    let mut artifacts = None;
    let mut case_id = None;
    let mut suite_id = None;
    let mut full = false;
    let mut jobs = 1_usize;
    let mut quality_manifest = None;
    let mut quality_profile = None;
    let mut json = false;
    let mut index = 0;

    while index < arguments.len() {
        match arguments[index].as_str() {
            "--manifest" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return test_suite_usage_error("--manifest requires a path");
                };
                manifest = Some(path.clone());
            }
            "--repo-root" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return test_suite_usage_error("--repo-root requires a path");
                };
                repo_root = Some(path.clone());
            }
            "--artifacts" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return test_suite_usage_error("--artifacts requires a path");
                };
                artifacts = Some(path.clone());
            }
            "--case" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return test_suite_usage_error("--case requires an ID");
                };
                case_id = Some(value.clone());
            }
            "--suite" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return test_suite_usage_error("--suite requires an ID");
                };
                suite_id = Some(value.clone());
            }
            "--full" => full = true,
            "--jobs" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return test_suite_usage_error("--jobs requires a positive integer");
                };
                jobs = match value.parse::<usize>() {
                    Ok(jobs) => jobs,
                    Err(_) => {
                        return test_suite_usage_error("--jobs requires a positive integer");
                    }
                };
            }
            "--quality-manifest" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return test_suite_usage_error("--quality-manifest requires a path");
                };
                quality_manifest = Some(path.clone());
            }
            "--quality-profile" => {
                index += 1;
                let Some(profile) = arguments.get(index) else {
                    return test_suite_usage_error("--quality-profile requires a name");
                };
                quality_profile = Some(profile.clone());
            }
            "--json" => json = true,
            "--help" | "-h" => {
                return CommandOutput {
                    stdout: HELP.to_owned(),
                    stderr: String::new(),
                    status: 0,
                };
            }
            unsupported => {
                return test_suite_usage_error(&format!(
                    "unsupported test argument: {unsupported}"
                ));
            }
        }
        index += 1;
    }

    if !json {
        return test_suite_usage_error("test requires --json");
    }
    let Some(manifest) = manifest else {
        return test_suite_usage_error("test requires --manifest PATH");
    };
    let repo_root = repo_root.unwrap_or_else(|| ".".to_owned());
    let selection =
        match test_suite::Selection::parse(case_id.as_deref(), suite_id.as_deref(), full) {
            Ok(selection) => selection,
            Err(error) => return test_suite_error(error),
        };
    let mut options = test_suite::RunnerOptions::new(manifest, repo_root)
        .with_selection(selection)
        .with_jobs(jobs);
    if let Some(artifacts) = artifacts {
        options = options.with_artifacts(artifacts);
    }
    if let Some(quality_manifest) = quality_manifest {
        options = options.with_quality_manifest(quality_manifest);
    }
    if let Some(quality_profile) = quality_profile {
        options = options.with_quality_profile(quality_profile);
    }
    match test_suite::run(&options) {
        Ok(report) => CommandOutput {
            stdout: serde_json::to_string(&report)
                .expect("test-suite report serialization is infallible")
                + "\n",
            stderr: String::new(),
            status: process_status_byte(report.exit_code),
        },
        Err(error) => test_suite_error(error),
    }
}

fn process_status_byte(status: i32) -> u8 {
    if (0..=255).contains(&status) {
        status as u8
    } else {
        1
    }
}

fn test_suite_usage_error(message: &str) -> CommandOutput {
    CommandOutput {
        stdout: String::new(),
        stderr: format!("omnirepo-dev: {message}\n"),
        status: 2,
    }
}

fn test_suite_error(error: test_suite::RunnerError) -> CommandOutput {
    CommandOutput {
        stdout: String::new(),
        stderr: format!("omnirepo-dev: {error}\n"),
        status: 2,
    }
}

fn run_coverage_ownership(arguments: &[String]) -> CommandOutput {
    let mut repository_root = None;
    let mut lcov = None;
    let mut matrix = None;
    let mut ownership = None;
    let mut json = false;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--repo-root" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return coverage_usage_error("coverage-ownership --repo-root requires a path");
                };
                repository_root = Some(path.clone());
            }
            "--lcov" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return coverage_usage_error("coverage-ownership --lcov requires a path");
                };
                lcov = Some(path.clone());
            }
            "--matrix" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return coverage_usage_error("coverage-ownership --matrix requires a path");
                };
                matrix = Some(path.clone());
            }
            "--ownership" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return coverage_usage_error("coverage-ownership --ownership requires a path");
                };
                ownership = Some(path.clone());
            }
            "--json" => json = true,
            "--help" | "-h" => {
                return CommandOutput {
                    stdout: HELP.to_owned(),
                    stderr: String::new(),
                    status: 0,
                };
            }
            unsupported => {
                return coverage_usage_error(&format!(
                    "unsupported coverage-ownership argument: {unsupported}"
                ));
            }
        }
        index += 1;
    }
    if !json {
        return coverage_usage_error("coverage-ownership requires --json");
    }
    let Some(repository_root) = repository_root else {
        return coverage_usage_error("coverage-ownership requires --repo-root PATH");
    };
    let Some(lcov) = lcov else {
        return coverage_usage_error("coverage-ownership requires --lcov PATH");
    };
    let Some(matrix) = matrix else {
        return coverage_usage_error("coverage-ownership requires --matrix PATH");
    };
    let Some(ownership) = ownership else {
        return coverage_usage_error("coverage-ownership requires --ownership PATH");
    };
    match coverage::attribute_repository(
        std::path::Path::new(&repository_root),
        std::path::Path::new(&lcov),
        std::path::Path::new(&matrix),
        std::path::Path::new(&ownership),
    ) {
        Ok(report) => match report.json() {
            Ok(json_report) => CommandOutput {
                stdout: json_report + "\n",
                stderr: String::new(),
                status: 0,
            },
            Err(error) => coverage_error(error.to_string()),
        },
        Err(error) => coverage_error(error.to_string()),
    }
}

fn coverage_usage_error(message: &str) -> CommandOutput {
    CommandOutput {
        stdout: String::new(),
        stderr: format!("omnirepo-dev: {message}\n"),
        status: 2,
    }
}

fn coverage_error(message: String) -> CommandOutput {
    CommandOutput {
        stdout: String::new(),
        stderr: format!("omnirepo-dev: coverage ownership failed: {message}\n"),
        status: 1,
    }
}

fn run_changed_coverage(arguments: &[String]) -> CommandOutput {
    let mut repository_root = None;
    let mut lcov = None;
    let mut base = None;
    let mut head = None;
    let mut report = None;
    let mut json = false;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--repo-root" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return changed_coverage_usage_error(
                        "changed-coverage --repo-root requires a path",
                    );
                };
                repository_root = Some(path.clone());
            }
            "--lcov" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return changed_coverage_usage_error("changed-coverage --lcov requires a path");
                };
                lcov = Some(path.clone());
            }
            "--base" => {
                index += 1;
                let Some(revision) = arguments.get(index) else {
                    return changed_coverage_usage_error(
                        "changed-coverage --base requires a revision",
                    );
                };
                base = Some(revision.clone());
            }
            "--head" => {
                index += 1;
                let Some(revision) = arguments.get(index) else {
                    return changed_coverage_usage_error(
                        "changed-coverage --head requires a revision",
                    );
                };
                head = Some(revision.clone());
            }
            "--report" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return changed_coverage_usage_error(
                        "changed-coverage --report requires a path",
                    );
                };
                report = Some(path.clone());
            }
            "--json" => json = true,
            "--help" | "-h" => {
                return CommandOutput {
                    stdout: HELP.to_owned(),
                    stderr: String::new(),
                    status: 0,
                };
            }
            unsupported => {
                return changed_coverage_usage_error(&format!(
                    "unsupported changed-coverage argument: {unsupported}"
                ));
            }
        }
        index += 1;
    }
    if !json {
        return changed_coverage_usage_error("changed-coverage requires --json");
    }
    let Some(repository_root) = repository_root else {
        return changed_coverage_usage_error("changed-coverage requires --repo-root PATH");
    };
    let Some(lcov) = lcov else {
        return changed_coverage_usage_error("changed-coverage requires --lcov PATH");
    };
    let base = match changed_coverage::resolve_base(base.as_deref()) {
        Ok(base) => base,
        Err(error) => return changed_coverage_error(error.to_string()),
    };
    let options = changed_coverage::Options {
        repository_root: std::path::PathBuf::from(repository_root),
        base,
        head,
        lcov_path: std::path::PathBuf::from(lcov),
        report_path: report.map(std::path::PathBuf::from),
    };
    match changed_coverage::evaluate(&options) {
        Ok(result) => match result.json() {
            Ok(json_report) => CommandOutput {
                stdout: json_report + "\n",
                stderr: String::new(),
                status: u8::from(!result.passed),
            },
            Err(error) => changed_coverage_error(error.to_string()),
        },
        Err(error) => changed_coverage_error(error.to_string()),
    }
}

fn changed_coverage_usage_error(message: &str) -> CommandOutput {
    CommandOutput {
        stdout: String::new(),
        stderr: format!("omnirepo-dev: {message}\n"),
        status: 2,
    }
}

fn changed_coverage_error(message: String) -> CommandOutput {
    CommandOutput {
        stdout: String::new(),
        stderr: format!("omnirepo-dev: changed coverage failed: {message}\n"),
        status: 1,
    }
}

fn run_viewer(arguments: &[String]) -> CommandOutput {
    let Some((command, arguments)) = arguments.split_first() else {
        return viewer_usage_error("viewer requires the refresh subcommand");
    };
    if command != "refresh" {
        return viewer_usage_error(&format!("unsupported viewer command: {command}"));
    }

    let mut input = None;
    let mut json = false;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--input" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return viewer_usage_error("--input requires a path");
                };
                input = Some(path.clone());
            }
            "--json" => json = true,
            "--help" | "-h" => {
                return CommandOutput {
                    stdout: HELP.to_owned(),
                    stderr: String::new(),
                    status: 0,
                };
            }
            unsupported => {
                return viewer_usage_error(&format!(
                    "unsupported viewer refresh argument: {unsupported}"
                ));
            }
        }
        index += 1;
    }

    if !json {
        return viewer_usage_error("viewer refresh requires --json");
    }
    let Some(input) = input else {
        return viewer_usage_error("viewer refresh requires --input PATH");
    };

    let source = match std::fs::read_to_string(&input) {
        Ok(source) => source,
        Err(error) => {
            return viewer_error(format!("viewer export cannot be read at {input}: {error}"));
        }
    };
    let export = match serde_json::from_str::<viewer::ViewerExport>(&source) {
        Ok(export) => export,
        Err(error) => {
            return viewer_error(format!("viewer export is invalid JSON: {error}"));
        }
    };
    let projection = match viewer::adapt_export(export) {
        Ok(projection) => projection,
        Err(error) => return viewer_error(format!("viewer export is invalid: {error}")),
    };

    CommandOutput {
        stdout: serde_json::to_string(&projection)
            .expect("viewer projection serialization is infallible")
            + "\n",
        stderr: String::new(),
        status: 0,
    }
}

fn viewer_usage_error(message: &str) -> CommandOutput {
    CommandOutput {
        stdout: String::new(),
        stderr: format!("omnirepo-dev: {message}\n"),
        status: 2,
    }
}

fn viewer_error(message: String) -> CommandOutput {
    CommandOutput {
        stdout: String::new(),
        stderr: format!("omnirepo-dev: {message}\n"),
        status: 1,
    }
}

/// Dispatch a command as a compact library result for unit callers.
///
/// The binary entry point uses [`run`] so it can preserve stdout, stderr, and
/// the validator's exit status independently.
pub fn dispatch<I, S>(arguments: I) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let output = run(arguments);
    if output.status == 0 {
        Ok(output.stdout)
    } else if !output.stderr.is_empty() {
        Err(output.stderr.trim_end().to_owned())
    } else {
        Err(output.stdout.trim_end().to_owned())
    }
}

fn run_validate_decisions(arguments: &[String]) -> CommandOutput {
    let mut json = false;
    for argument in arguments {
        match argument.as_str() {
            "--json" | "-j" => json = true,
            "--help" | "-h" => {
                return CommandOutput {
                    stdout: HELP.to_owned(),
                    stderr: String::new(),
                    status: 0,
                };
            }
            unsupported => {
                return CommandOutput {
                    stdout: String::new(),
                    stderr: format!(
                        "omnirepo-dev: unsupported validate-decisions argument: {unsupported}\n"
                    ),
                    status: 2,
                };
            }
        }
    }

    let report = match beads_validator::validate_default() {
        Ok(report) => report,
        Err(error) => error.into_report(),
    };
    if json {
        return CommandOutput {
            stdout: serde_json::to_string_pretty(&report)
                .expect("validation report serialization is infallible")
                + "\n",
            stderr: String::new(),
            status: if report.is_valid() { 0 } else { 1 },
        };
    }

    if report.is_valid() {
        CommandOutput {
            stdout: "decision workflow is consistent\n".to_owned(),
            stderr: String::new(),
            status: 0,
        }
    } else {
        CommandOutput {
            stdout: String::new(),
            stderr: render_text_report(&report),
            status: 1,
        }
    }
}

fn render_text_report(report: &beads_validator::ValidationReport) -> String {
    let mut output = String::new();
    if let Some(first) = report.findings.first() {
        if matches!(
            first.code,
            beads_validator::FindingCode::TrackedJsonlMissing
                | beads_validator::FindingCode::TrackedJsonlEmpty
                | beads_validator::FindingCode::TrackedJsonlUnreadable
                | beads_validator::FindingCode::TrackedJsonlInvalidUtf8
        ) {
            output.push_str("decision workflow invalid: ");
            output.push_str(&first.message);
            output.push_str(": ");
            output.push_str(&report.path);
            output.push('\n');
            return output;
        }
    }

    output.push_str("decision workflow invalid: ");
    output.push_str(&report.path);
    output.push('\n');
    for finding in &report.findings {
        output.push_str("  ");
        if let Some(line) = finding.line {
            output.push_str(&format!("line {line} "));
        }
        output.push_str("issue ");
        let issue_id = finding
            .issue_id
            .as_ref()
            .map(|issue_id| issue_id.as_str())
            .unwrap_or("<missing>");
        output.push_str(
            &serde_json::to_string(issue_id).expect("issue identifier serialization is infallible"),
        );
        output.push_str(": ");
        output.push_str(&finding.message);
        output.push('\n');
    }
    if report.truncated {
        output.push_str("  diagnostics truncated at 64 findings\n");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{HELP, VERSION, dispatch};

    #[test]
    fn empty_and_help_invocations_are_safe() {
        assert_eq!(dispatch(Vec::<String>::new()), Ok(HELP.to_owned()));
        assert_eq!(dispatch(["--help"]), Ok(HELP.to_owned()));
        assert_eq!(dispatch(["-h"]), Ok(HELP.to_owned()));
    }

    #[test]
    fn version_invocations_report_the_package_version() {
        let expected = format!("omnirepo-dev {VERSION}\n");

        assert_eq!(dispatch(["--version"]), Ok(expected.clone()));
        assert_eq!(dispatch(["-V"]), Ok(expected));
    }

    #[test]
    fn unsupported_commands_fail_closed() {
        assert_eq!(
            dispatch(["future-command"]),
            Err("omnirepo-dev: unsupported developer command: future-command".to_owned())
        );
    }
}

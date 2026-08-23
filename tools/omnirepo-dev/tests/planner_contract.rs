//! Frozen checked-plan and process-boundary contracts for `omnirepo-dev`.
//!
//! These tests use fixture-local fake `br` executables.  The adapter still
//! receives an executable path, repository root, closed stdin, and no shell
//! command string.  No test points at or mutates the live `.beads` database.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use omnirepo_dev::br_adapter::{
    BrAdapter, BrAdapterConfig, BrAdapterError, ProcessDiagnostics, SourceKind,
};
use omnirepo_dev::planner::{PlanStatus, PlannerInputs, plan, report_for_adapter_error};
use serde::Deserialize;
use serde_json::Value;

static NEXT_ROOT: AtomicUsize = AtomicUsize::new(0);
static PATH_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Deserialize)]
struct FrozenCase {
    tracked_lines: Vec<String>,
    #[serde(default, rename = "omit_tracked")]
    _omit_tracked: bool,
    ready: Value,
    scheduler: Value,
}

#[derive(Debug, Deserialize)]
struct FrozenGolden {
    case_id: String,
    exit_status: i32,
    stream: String,
    output: FrozenOutput,
}

#[derive(Debug, Deserialize)]
struct FrozenOutput {
    schema: String,
    status: String,
    #[serde(default)]
    candidate_ids: Vec<String>,
    #[serde(default)]
    excluded: Vec<FrozenExclusion>,
    error_code: Option<String>,
    #[serde(default)]
    error_issues: Vec<FrozenIssue>,
}

#[derive(Debug, Deserialize)]
struct FrozenExclusion {
    id: String,
    reason: String,
}

#[derive(Debug, Deserialize)]
struct FrozenIssue {
    id: String,
    reason: String,
}

fn fixture_case(name: &str) -> FrozenCase {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/beads_contract/cases")
        .join(format!("{name}.json"));
    serde_json::from_str(&fs::read_to_string(path).expect("read frozen planner case"))
        .expect("parse frozen planner case")
}

fn fixture_golden(name: &str) -> FrozenGolden {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/beads_contract/golden")
        .join(format!("{name}.json"));
    serde_json::from_str(&fs::read_to_string(path).expect("read frozen planner golden"))
        .expect("parse frozen planner golden")
}

fn root(name: &str) -> PathBuf {
    let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "omnirepo-dev-planner-{name}-{}-{sequence}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(root.join(".beads")).expect("create fixture root");
    root
}

fn cleanup(root: &Path) {
    if root.exists() {
        fs::remove_dir_all(root).expect("remove fixture root");
    }
}

fn write_tracked(root: &Path, lines: &[String]) -> PathBuf {
    let path = root.join(".beads/issues.jsonl");
    let mut contents = lines.join("\n");
    if !contents.is_empty() {
        contents.push('\n');
    }
    fs::write(&path, contents).expect("write tracked export");
    path
}

fn write_sources(root: &Path, ready: &Value, scheduler: &Value) {
    fs::write(
        root.join("ready.json"),
        serde_json::to_vec(ready).expect("serialize ready fixture"),
    )
    .expect("write ready fixture");
    fs::write(
        root.join("scheduler.json"),
        serde_json::to_vec(scheduler).expect("serialize scheduler fixture"),
    )
    .expect("write scheduler fixture");
}

fn fake_br(root: &Path, mode: &str) -> PathBuf {
    let executable = root.join("fake-br");
    let script = format!(
        "#!/bin/sh\nset -eu\nif [ \"$1\" != \"--no-auto-import\" ] || [ \"$2\" != \"--no-auto-flush\" ] || [ \"$4\" != \"--json\" ]; then exit 41; fi\nif read _line; then exit 42; fi\nprintf '%s\\n' \"${{BEADS_JSONL-<unset>}}\" > \"$PWD/seen-beads-jsonl\"\nprintf '%s\\n' \"$PWD\" > \"$PWD/seen-cwd-$3\"\ncase \"{mode}:$3\" in\n  nonzero:*) printf '%s\\n' 'fake br failure' >&2; exit 7 ;;\n  stderr-oversized:*) /usr/bin/yes x >&2 ;;\n  invalid-utf8:ready) /usr/bin/printf '\\377\\n' ;;\n  invalid-utf8-stderr:ready) /usr/bin/printf '[]\\n'; /usr/bin/printf '\\377\\n' >&2 ;;\n  signal:*) kill -TERM $$ ;;\n  malformed:ready) printf '%s\\n' '{{}}' ;;\n  malformed:scheduler) printf '%s\\n' '[]' ;;\n  timeout:*) while :; do :; done ;;\n  oversized:*) /usr/bin/yes x ;;\n  late:ready) (/bin/sleep 4) & exit 0 ;;\n  *:ready) /bin/cat \"$PWD/ready.json\" ;;\n  *:scheduler) /bin/cat \"$PWD/scheduler.json\" ;;\n  *) exit 43 ;;\nesac\n",
        mode = mode
    );
    // Publish atomically: a concurrent exec must never see a
    // half-written script (ETXTBSY).
    let temporary = root.join(format!(".fake-br.tmp-{}", std::process::id()));
    fs::write(&temporary, &script).expect("write fake br temp");
    set_executable(&temporary);
    fs::rename(&temporary, &executable).expect("publish fake br");
    executable
}

fn profile_fake_br(root: &Path) -> PathBuf {
    let executable = root.join("profile-fake-br");
    let script = r#"#!/bin/sh
set -eu
if [ "$1" != "--no-auto-import" ] || [ "$2" != "--no-auto-flush" ] || [ "$4" != "--json" ]; then exit 41; fi
printf '%s\n' "${LLVM_PROFILE_FILE-<unset>}" > "$PWD/seen-profile"
printf '[]\n'
"#;
    // Publish atomically: a concurrent exec must never see a
    // half-written script (ETXTBSY).
    let temporary = root.join(format!(".profile-fake-br.tmp-{}", std::process::id()));
    fs::write(&temporary, script).expect("write profile fake br temp");
    set_executable(&temporary);
    fs::rename(&temporary, &executable).expect("publish profile fake br");
    executable
}

fn set_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path).expect("fake br metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("chmod fake br");
    }
}

fn adapter(root: &Path, mode: &str) -> BrAdapter {
    let executable = fake_br(root, mode);
    let config =
        BrAdapterConfig::with_executable(root, executable).expect("freeze fake br identity");
    BrAdapter::new(config)
}

struct PathGuard {
    previous: Option<OsString>,
}

impl Drop for PathGuard {
    fn drop(&mut self) {
        // SAFETY: tests serialize all PATH mutations with PATH_LOCK and restore
        // the caller's original value before releasing the lock.
        unsafe {
            match self.previous.take() {
                Some(path) => std::env::set_var("PATH", path),
                None => std::env::remove_var("PATH"),
            }
        }
    }
}

fn with_path<T>(path: Option<&Path>, operation: impl FnOnce() -> T) -> T {
    let _lock = PATH_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("PATH lock is not poisoned");
    let guard = PathGuard {
        previous: std::env::var_os("PATH"),
    };
    // SAFETY: PATH is changed only while PATH_LOCK is held.  The guard restores
    // the previous value even when the operation panics.
    unsafe {
        match path {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
    }
    let result = operation();
    drop(guard);
    result
}

struct ProfileEnvironmentGuard {
    previous: Option<OsString>,
}

impl Drop for ProfileEnvironmentGuard {
    fn drop(&mut self) {
        // SAFETY: the caller holds PATH_LOCK while the process environment is
        // changed and restored.
        unsafe {
            match self.previous.take() {
                Some(profile) => std::env::set_var("LLVM_PROFILE_FILE", profile),
                None => std::env::remove_var("LLVM_PROFILE_FILE"),
            }
        }
    }
}

fn with_llvm_profile<T>(profile: &str, operation: impl FnOnce() -> T) -> T {
    let _lock = PATH_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("environment lock is not poisoned");
    let _guard = ProfileEnvironmentGuard {
        previous: std::env::var_os("LLVM_PROFILE_FILE"),
    };
    // SAFETY: LLVM_PROFILE_FILE is changed only while PATH_LOCK is held.  The
    // guard restores the caller's value even when the operation panics.
    unsafe {
        std::env::set_var("LLVM_PROFILE_FILE", profile);
    }
    operation()
}

fn valid_tracked() -> &'static str {
    r#"{"id":"normal","status":"open","labels":[]}
"#
}

fn plan_value(ready: Value, scheduler: Value, tracked: &str) -> Value {
    let ready_json = serde_json::to_string(&ready).expect("serialize ready JSON");
    let scheduler_json = serde_json::to_string(&scheduler).expect("serialize scheduler JSON");
    serde_json::to_value(plan(PlannerInputs {
        ready_json: &ready_json,
        scheduler_json: &scheduler_json,
        tracked_jsonl: tracked,
        tracked_path: Path::new("fixture/.beads/issues.jsonl"),
    }))
    .expect("serialize planner report")
}

fn assert_error_code(report: &Value, code: &str) {
    assert_eq!(report["status"], "error");
    assert_eq!(report["error"]["code"], code);
    assert!(report["error"]["issues"].is_array());
}

fn plan_for_case(name: &str) -> Value {
    let case = fixture_case(name);
    let tracked = case.tracked_lines.join("\n")
        + if case.tracked_lines.is_empty() {
            ""
        } else {
            "\n"
        };
    let ready = serde_json::to_string(&case.ready).expect("serialize ready");
    let scheduler = serde_json::to_string(&case.scheduler).expect("serialize scheduler");
    let report = plan(PlannerInputs {
        ready_json: &ready,
        scheduler_json: &scheduler,
        tracked_jsonl: &tracked,
        tracked_path: Path::new("fixture/.beads/issues.jsonl"),
    });
    serde_json::to_value(report).expect("serialize checked plan")
}

fn candidate_ids(report: &Value) -> Vec<String> {
    report["candidates"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["id"].as_str().map(ToOwned::to_owned))
        .collect()
}

fn exclusions(report: &Value) -> Vec<(String, String)> {
    report["excluded"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            Some((
                item["id"].as_str()?.to_owned(),
                item["reason"].as_str()?.to_owned(),
            ))
        })
        .collect()
}

#[test]
fn frozen_planner_cases_preserve_schema_candidates_exclusions_and_errors() {
    let cases = [
        "planner-valid-exclusions",
        "planner-invalid-decision-drift",
        "planner-tracked-status-disagreement",
        "planner-tracked-label-disagreement",
        "planner-missing-tracked",
        "planner-set-disagreement",
        "planner-duplicate-ready",
        "planner-scheduler-status-disagreement",
        "planner-scheduler-label-disagreement",
        "planner-ready-malformed",
        "planner-scheduler-malformed",
        "planner-tracked-malformed",
        "planner-empty-tracked-export",
    ];

    for name in cases {
        let report = plan_for_case(name);
        let golden = fixture_golden(name);
        assert_eq!(golden.case_id, name);
        assert_eq!(report["schema"], golden.output.schema, "case {name}");
        assert_eq!(report["status"], golden.output.status, "case {name}");
        if golden.exit_status == 0 {
            assert_eq!(golden.stream, "stdout");
            let actual_candidates = candidate_ids(&report);
            assert_eq!(
                actual_candidates, golden.output.candidate_ids,
                "case {name}"
            );
            let actual_excluded = exclusions(&report);
            let expected_excluded = golden
                .output
                .excluded
                .into_iter()
                .map(|item| (item.id, item.reason))
                .collect::<Vec<_>>();
            assert_eq!(actual_excluded, expected_excluded, "case {name}");
        } else {
            assert_eq!(golden.stream, "stderr");
            assert_eq!(
                report["error"]["code"].as_str(),
                golden.output.error_code.as_deref(),
                "case {name}"
            );
            let actual_issues = report["error"]["issues"]
                .as_array()
                .expect("error issues array")
                .iter()
                .map(|item| {
                    (
                        item["id"].as_str().unwrap_or_default().to_owned(),
                        item["reason"].as_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect::<Vec<_>>();
            let expected_issues = golden
                .output
                .error_issues
                .into_iter()
                .map(|item| (item.id, item.reason))
                .collect::<Vec<_>>();
            assert_eq!(actual_issues, expected_issues, "case {name}");
        }
    }
}

#[test]
fn adapter_uses_exact_read_only_commands_fixed_cwd_and_sanitized_environment() {
    let root = root("success");
    let case = fixture_case("planner-valid-exclusions");
    write_sources(&root, &case.ready, &case.scheduler);
    let tracked = write_tracked(&root, &case.tracked_lines);
    let tracked_before = fs::read(&tracked).expect("tracked before");
    let ready_before = fs::read(root.join("ready.json")).expect("ready before");
    let scheduler_before = fs::read(root.join("scheduler.json")).expect("scheduler before");

    let report = helpers::run_planner(&root, adapter(&root, "normal"), tracked.clone());
    assert_eq!(report.status, PlanStatus::Ok);
    let json = serde_json::to_value(report).expect("serialize success report");
    assert_eq!(candidate_ids(&json), vec!["normal-work"]);
    assert_eq!(
        exclusions(&json),
        vec![
            ("decision-active".to_owned(), "owner-decision".to_owned()),
            (
                "decision-closed".to_owned(),
                "closed-decision-history".to_owned()
            ),
            ("blocked-work".to_owned(), "blocked".to_owned()),
            ("deferred-work".to_owned(), "deferred".to_owned()),
            ("container-work".to_owned(), "container".to_owned()),
        ]
    );
    let expected_root = root.canonicalize().expect("canonical root");
    assert_eq!(
        fs::read_to_string(root.join("seen-cwd-ready")).expect("ready cwd"),
        format!("{}\n", expected_root.display())
    );
    assert_eq!(
        fs::read_to_string(root.join("seen-cwd-scheduler")).expect("scheduler cwd"),
        format!("{}\n", expected_root.display())
    );
    assert_eq!(
        fs::read_to_string(root.join("seen-beads-jsonl")).expect("sanitized tracker env"),
        "<unset>\n"
    );
    assert_eq!(fs::read(&tracked).expect("tracked after"), tracked_before);
    assert_eq!(
        fs::read(root.join("ready.json")).expect("ready after"),
        ready_before
    );
    assert_eq!(
        fs::read(root.join("scheduler.json")).expect("scheduler after"),
        scheduler_before
    );
    cleanup(&root);
}

#[test]
fn adapter_preserves_llvm_profile_file_through_sanitized_environment() {
    let root = root("profile-environment");
    let executable = profile_fake_br(&root);
    let config =
        BrAdapterConfig::with_executable(&root, executable).expect("freeze profile fake br");
    let adapter = BrAdapter::new(config);

    let output = with_llvm_profile("profile-%p.profraw", || {
        adapter.ready().expect("profile fake br should run")
    });
    assert_eq!(output.stdout, "[]\n");
    assert_eq!(
        fs::read_to_string(root.join("seen-profile")).expect("profile environment evidence"),
        "profile-%p.profraw\n"
    );
    cleanup(&root);
}

#[test]
fn adapter_classifies_nonzero_missing_and_incompatible_executables() {
    let root = root("errors");
    let missing = root.join("missing-br");
    assert!(matches!(
        BrAdapterConfig::with_executable(&root, &missing),
        Err(BrAdapterError::MissingExecutable { .. })
    ));

    let nonzero = adapter(&root, "nonzero");
    let error = nonzero.ready().expect_err("nonzero fake br must fail");
    assert!(matches!(
        error,
        BrAdapterError::NonZero {
            source: SourceKind::Ready,
            ..
        }
    ));
    assert_eq!(error.reason_code(), "canonical-source-command-failed");

    let directory = root.join("directory-br");
    fs::create_dir(&directory).expect("create incompatible executable path");
    assert!(matches!(
        BrAdapterConfig::with_executable(&root, &directory),
        Err(BrAdapterError::IncompatibleExecutable { .. })
    ));
    cleanup(&root);
}

#[test]
fn malformed_source_is_reported_without_falling_back_to_viewer() {
    let root = root("malformed");
    let case = fixture_case("planner-valid-exclusions");
    let tracked = write_tracked(&root, &case.tracked_lines);
    let malformed_ready = serde_json::json!({"not":"ready"});
    write_sources(&root, &malformed_ready, &case.scheduler);
    let report = helpers::run_planner(&root, adapter(&root, "malformed"), tracked);
    let json = serde_json::to_value(report).expect("serialize malformed report");
    assert_eq!(json["status"], "error");
    assert_eq!(json["error"]["code"], "canonical-source-malformed");
    assert_eq!(json["error"]["issues"][0]["id"], "<ready>");
    assert_eq!(
        json["error"]["issues"][0]["reason"],
        "canonical-ready-malformed"
    );
    assert!(
        !serde_json::to_string(&json)
            .expect("serialize JSON")
            .contains("bv")
    );
    cleanup(&root);
}

#[test]
fn timeout_and_oversized_output_terminate_the_process_tree() {
    let extreme_root = root("extreme-timeout");
    let extreme = BrAdapterConfig::with_executable(&extreme_root, fake_br(&extreme_root, "normal"))
        .expect("freeze extreme-timeout fake")
        .with_timeout(Duration::MAX);
    let extreme_error = BrAdapter::new(extreme)
        .ready()
        .expect_err("an unrepresentable timeout must be rejected");
    assert!(matches!(
        extreme_error,
        BrAdapterError::InvalidTimeout {
            timeout: Duration::MAX,
            ..
        }
    ));
    assert!(!extreme_root.join("seen-cwd-ready").exists());
    cleanup(&extreme_root);

    let timeout_root = root("timeout");
    let timeout =
        BrAdapterConfig::with_executable(&timeout_root, fake_br(&timeout_root, "timeout"))
            .expect("freeze timeout fake")
            .with_timeout(Duration::from_millis(40));
    let started = Instant::now();
    let timeout_error = BrAdapter::new(timeout)
        .ready()
        .expect_err("timeout fake must fail");
    assert!(matches!(timeout_error, BrAdapterError::Timeout { .. }));
    // The guard proves the run was bounded at all, not that it was quick:
    // it must stay well clear of process-spawn cost under a loaded host,
    // while a genuine unbounded wait still reaches it.
    assert!(started.elapsed() < Duration::from_secs(30));
    cleanup(&timeout_root);

    let oversized_root = root("oversized");
    // The output bound is under test here, not the timeout, so the budget
    // is a margin that must never preempt the overflow: the fake exceeds
    // the byte limit immediately, and a budget near process-spawn cost
    // reports `Timeout` instead when the suite runs cases in parallel.
    // A generous budget costs nothing, because it never elapses.
    let oversized =
        BrAdapterConfig::with_executable(&oversized_root, fake_br(&oversized_root, "oversized"))
            .expect("freeze oversized fake")
            .with_timeout(Duration::from_secs(60))
            .with_max_output_bytes(64);
    let oversized_error = BrAdapter::new(oversized)
        .ready()
        .expect_err("oversized fake must fail");
    assert!(matches!(
        oversized_error,
        BrAdapterError::OutputTooLarge {
            stream: "stdout",
            limit: 64,
            ..
        }
    ));
    cleanup(&oversized_root);
}

#[test]
fn late_descendants_are_cleaned_and_concurrent_reads_do_not_share_state() {
    let late_root = root("late");
    let late = adapter(&late_root, "late");
    let started = Instant::now();
    late.ready().expect("late parent exits successfully");
    // Bounded, not quick: the parent must not be held by its late
    // descendant.  The margin clears spawn cost on a loaded host.
    assert!(started.elapsed() < Duration::from_secs(30));
    cleanup(&late_root);

    let concurrent_root = root("concurrent");
    let case = fixture_case("planner-valid-exclusions");
    write_sources(&concurrent_root, &case.ready, &case.scheduler);
    let concurrent_adapter = adapter(&concurrent_root, "normal");
    let workers = (0..8)
        .map(|_| {
            let adapter = concurrent_adapter.clone();
            thread::spawn(move || adapter.ready().expect("concurrent ready read"))
        })
        .collect::<Vec<_>>();
    for worker in workers {
        let output = worker.join().expect("join concurrent read");
        assert_eq!(output.source, SourceKind::Ready);
        assert!(output.stdout.contains("normal-work"));
    }
    cleanup(&concurrent_root);
}

#[test]
fn missing_tracked_state_fails_before_any_plan_is_emitted() {
    let root = root("missing-tracked");
    let case = fixture_case("planner-valid-exclusions");
    write_sources(&root, &case.ready, &case.scheduler);
    let tracked = root.join(".beads/issues.jsonl");
    let report = helpers::run_planner(&root, adapter(&root, "normal"), tracked);
    let json = serde_json::to_value(report).expect("serialize missing report");
    assert_eq!(json["status"], "error");
    assert_eq!(json["error"]["code"], "tracked-export-missing");
    assert!(json.get("candidates").is_none());
    assert!(!root.join("seen-cwd-ready").exists());
    cleanup(&root);
}

#[test]
fn adapter_freezes_path_discovered_executables_and_rejects_invalid_roots() {
    let root = root("discover");
    let bin = root.join("bin");
    fs::create_dir(&bin).expect("create PATH fixture");
    let source = fake_br(&root, "normal");
    let discovered_path = bin.join("br");
    fs::copy(&source, &discovered_path).expect("copy fake br into PATH fixture");
    set_executable(&discovered_path);

    let config = with_path(Some(&bin), || BrAdapterConfig::discover(&root))
        .expect("discover fake br from fixture PATH");
    assert_eq!(
        config.executable,
        discovered_path
            .canonicalize()
            .expect("canonical discovered executable")
    );
    assert!(matches!(
        with_path(Some(&root.join("empty-path")), || {
            fs::create_dir_all(root.join("empty-path")).expect("create empty PATH");
            BrAdapterConfig::discover(&root)
        }),
        Err(BrAdapterError::MissingExecutable { executable }) if executable == Path::new("br")
    ));

    let missing_root = root.join("missing-root");
    assert!(matches!(
        BrAdapterConfig::with_executable(&missing_root, &source),
        Err(BrAdapterError::InvalidRepositoryRoot { .. })
    ));
    let file_root = root.join("file-root");
    fs::write(&file_root, b"not a directory").expect("write file root");
    assert!(matches!(
        BrAdapterConfig::with_executable(&file_root, &source),
        Err(BrAdapterError::InvalidRepositoryRoot { .. })
    ));
    cleanup(&root);
}

#[test]
fn adapter_classifies_spawn_utf8_stderr_and_signal_failures() {
    let spawn_root = root("spawn");
    let spawn_executable = fake_br(&spawn_root, "normal");
    let spawn_config = BrAdapterConfig::with_executable(&spawn_root, &spawn_executable)
        .expect("freeze spawn fake");
    fs::remove_file(&spawn_executable).expect("remove frozen executable");
    let spawn_error = BrAdapter::new(spawn_config)
        .ready()
        .expect_err("removed frozen executable must fail to spawn");
    assert!(matches!(
        spawn_error,
        BrAdapterError::Spawn {
            source: SourceKind::Ready,
            ..
        }
    ));
    cleanup(&spawn_root);

    let utf8_root = root("invalid-utf8");
    let utf8_error = adapter(&utf8_root, "invalid-utf8")
        .ready()
        .expect_err("invalid UTF-8 stdout must fail closed");
    assert!(matches!(
        utf8_error,
        BrAdapterError::InvalidUtf8 {
            source: SourceKind::Ready,
            stream: "stdout",
            ..
        }
    ));
    cleanup(&utf8_root);

    let stderr_root = root("invalid-utf8-stderr");
    let stderr_error = adapter(&stderr_root, "invalid-utf8-stderr")
        .ready()
        .expect_err("invalid UTF-8 stderr must fail closed");
    assert!(matches!(
        stderr_error,
        BrAdapterError::InvalidUtf8 {
            source: SourceKind::Ready,
            stream: "stderr",
            ..
        }
    ));
    cleanup(&stderr_root);

    let signal_root = root("signal");
    let signal_error = adapter(&signal_root, "signal")
        .ready()
        .expect_err("signal exit must fail closed");
    assert!(matches!(
        signal_error,
        BrAdapterError::NonZero {
            source: SourceKind::Ready,
            status,
            ..
        } if status.starts_with("signal:")
    ));
    cleanup(&signal_root);
}

#[test]
fn adapter_bounds_stderr_and_preserves_bounded_diagnostics() {
    let root = root("stderr-oversized");
    let error = BrAdapter::new(
        BrAdapterConfig::with_executable(&root, fake_br(&root, "stderr-oversized"))
            .expect("freeze stderr overflow fake")
            // The stderr bound is under test, not the timeout: the budget
            // must never preempt the overflow.  It never elapses.
            .with_timeout(Duration::from_secs(60))
            .with_max_output_bytes(64),
    )
    .ready()
    .expect_err("oversized stderr must fail closed");
    assert!(matches!(
        error,
        BrAdapterError::OutputTooLarge {
            source: SourceKind::Ready,
            stream: "stderr",
            limit: 64,
            diagnostics,
        } if diagnostics.stdout.is_empty() && diagnostics.stderr.len() <= 256
    ));
    cleanup(&root);
}

#[test]
fn adapter_error_projections_are_stable_for_every_variant() {
    let diagnostics = ProcessDiagnostics {
        stdout: "out".to_owned(),
        stderr: "err".to_owned(),
    };
    let errors = vec![
        BrAdapterError::InvalidRepositoryRoot {
            path: PathBuf::from("/bad/root"),
            reason: "not a directory".to_owned(),
        },
        BrAdapterError::MissingExecutable {
            executable: PathBuf::from("br"),
        },
        BrAdapterError::IncompatibleExecutable {
            executable: PathBuf::from("/bad/br"),
            reason: "not executable".to_owned(),
        },
        BrAdapterError::InvalidTimeout {
            timeout: Duration::MAX,
            reason: "deadline would overflow the platform clock".to_owned(),
        },
        BrAdapterError::Spawn {
            source: SourceKind::Ready,
            command: "br ready --json".to_owned(),
            reason: "permission denied".to_owned(),
        },
        BrAdapterError::NonZero {
            source: SourceKind::Scheduler,
            status: "code:7".to_owned(),
            diagnostics: diagnostics.clone(),
        },
        BrAdapterError::Timeout {
            source: SourceKind::Ready,
            timeout: Duration::from_secs(2),
            diagnostics: diagnostics.clone(),
        },
        BrAdapterError::OutputTooLarge {
            source: SourceKind::Scheduler,
            stream: "stderr",
            limit: 64,
            diagnostics: diagnostics.clone(),
        },
        BrAdapterError::InvalidUtf8 {
            source: SourceKind::Ready,
            stream: "stdout",
            diagnostics: diagnostics.clone(),
        },
        BrAdapterError::Read {
            source: SourceKind::Scheduler,
            stream: "stderr",
            reason: "synthetic reader failure".to_owned(),
            diagnostics: diagnostics.clone(),
        },
        BrAdapterError::Wait {
            source: SourceKind::Scheduler,
            reason: "wait failed".to_owned(),
            diagnostics,
        },
    ];
    let expected_codes = [
        "invalid-repository-root",
        "required-command-missing",
        "incompatible-command",
        "invalid-timeout",
        "canonical-source-command-failed",
        "canonical-source-command-failed",
        "canonical-source-timeout",
        "canonical-source-output-too-large",
        "canonical-source-invalid-utf8",
        "canonical-source-read-failed",
        "canonical-source-wait-failed",
    ];
    let expected_report_codes = [
        "invalid-repository-root",
        "required-command-missing",
        "incompatible-command",
        "invalid-timeout",
        "canonical-source-failure",
        "canonical-source-failure",
        "canonical-source-timeout",
        "canonical-source-output-too-large",
        "canonical-source-invalid-utf8",
        "canonical-source-read-failed",
        "canonical-source-wait-failed",
    ];
    for ((error, expected_code), expected_report_code) in errors
        .into_iter()
        .zip(expected_codes)
        .zip(expected_report_codes)
    {
        let display = error.to_string();
        assert!(!display.is_empty());
        assert_eq!(error.reason_code(), expected_code);
        assert!(std::error::Error::source(&error).is_none());
        let expected_source = match &error {
            BrAdapterError::InvalidRepositoryRoot { .. }
            | BrAdapterError::MissingExecutable { .. }
            | BrAdapterError::IncompatibleExecutable { .. }
            | BrAdapterError::InvalidTimeout { .. } => None,
            BrAdapterError::Spawn { source, .. }
            | BrAdapterError::NonZero { source, .. }
            | BrAdapterError::Timeout { source, .. }
            | BrAdapterError::OutputTooLarge { source, .. }
            | BrAdapterError::InvalidUtf8 { source, .. }
            | BrAdapterError::Read { source, .. }
            | BrAdapterError::Wait { source, .. } => Some(*source),
        };
        assert_eq!(error.source_kind(), expected_source);
        let has_diagnostics = error.diagnostics().is_some();
        assert_eq!(
            has_diagnostics,
            !matches!(
                error,
                BrAdapterError::InvalidRepositoryRoot { .. }
                    | BrAdapterError::MissingExecutable { .. }
                    | BrAdapterError::IncompatibleExecutable { .. }
                    | BrAdapterError::InvalidTimeout { .. }
                    | BrAdapterError::Spawn { .. }
            )
        );
        let report = serde_json::to_value(report_for_adapter_error(error))
            .expect("serialize adapter error report");
        assert_error_code(&report, expected_report_code);
    }
}

#[test]
fn planner_diagnostics_preserve_reader_stream_and_bounded_evidence() {
    let error = BrAdapterError::Read {
        source: SourceKind::Ready,
        stream: "stdout",
        reason: "synthetic reader failure".to_owned(),
        diagnostics: ProcessDiagnostics {
            stdout: "prefix evidence".to_owned(),
            stderr: "child diagnostic".to_owned(),
        },
    };
    let report = serde_json::to_value(report_for_adapter_error(error))
        .expect("serialize reader failure report");
    assert_eq!(report["status"], "error");
    assert_eq!(report["error"]["code"], "canonical-source-read-failed");
    let issue = &report["error"]["issues"][0];
    assert_eq!(issue["id"], "<ready>");
    assert_eq!(issue["reason"], "canonical-source-read-failed");
    assert!(
        issue["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("stdout")
    );
    assert_eq!(issue["stdout"], "prefix evidence");
    assert_eq!(issue["stderr"], "child diagnostic");
}

#[test]
fn planner_rejects_ready_issue_shape_variants_without_fallbacks() {
    let malformed_ready = [
        serde_json::json!([null]),
        serde_json::json!([{}]),
        serde_json::json!([{"id": 1, "status": "open", "labels": []}]),
        serde_json::json!([{"id": "", "status": "open", "labels": []}]),
        serde_json::json!([{"id": "x", "labels": []}]),
        serde_json::json!([{"id": "x", "status": 1, "labels": []}]),
        serde_json::json!([{"id": "x", "status": "open"}]),
        serde_json::json!([{"id": "x", "status": "open", "labels": {}}]),
        serde_json::json!([{"id": "x", "status": "open", "labels": [1]}]),
    ];
    for ready in malformed_ready {
        let report = plan_value(ready, serde_json::json!({"recommendations": []}), "");
        assert_error_code(&report, "canonical-source-malformed");
        assert_eq!(
            report["error"]["issues"][0]["reason"],
            "canonical-ready-malformed"
        );
    }
}

#[test]
fn planner_rejects_scheduler_shape_variants_without_viewer_fallback() {
    let malformed_scheduler = [
        serde_json::json!(null),
        serde_json::json!([]),
        serde_json::json!({}),
        serde_json::json!({"recommendations": {}}),
        serde_json::json!({"recommendations": [null]}),
        serde_json::json!({"recommendations": [{}]}),
        serde_json::json!({"recommendations": [{"issue": null}]}),
        serde_json::json!({
            "recommendations": [{"issue": {"id": 1, "status": "open", "labels": []}}]
        }),
    ];
    let ready = serde_json::json!([]);
    for scheduler in malformed_scheduler {
        let report = plan_value(ready.clone(), scheduler, "");
        assert_error_code(&report, "canonical-source-malformed");
        assert_eq!(
            report["error"]["issues"][0]["reason"],
            "canonical-scheduler-malformed"
        );
    }
}

#[test]
fn planner_accepts_missing_scheduler_rank_as_null_and_preserves_label_order_insensitivity() {
    let ready = serde_json::json!([{
        "id": "normal",
        "status": "open",
        "labels": ["alpha", "beta"]
    }]);
    let scheduler = serde_json::json!({
        "recommendations": [{
            "issue": {
                "id": "normal",
                "status": "open",
                "labels": ["beta", "alpha"]
            }
        }]
    });
    let tracked = r#"{"id":"normal","status":"open","labels":["beta","alpha"]}
"#;
    let report = plan_value(ready, scheduler, tracked);
    assert_eq!(report["status"], "ok");
    assert_eq!(report["candidates"][0]["id"], "normal");
    assert!(report["candidates"][0]["rank"].is_null());
}

#[test]
fn planner_excludes_epic_and_non_ready_statuses_with_stable_reasons() {
    let statuses = [
        ("in-progress", "in_progress", "status-not-ready"),
        ("tombstoned", "tombstone", "status-not-ready"),
    ];
    let mut ready_items = Vec::new();
    let mut scheduler_items = Vec::new();
    let mut tracked_lines = Vec::new();
    for (id, status, _) in statuses {
        ready_items.push(serde_json::json!({
            "id": id,
            "status": status,
            "labels": []
        }));
        scheduler_items.push(serde_json::json!({
            "rank": 1,
            "issue": {"id": id, "status": status, "labels": []}
        }));
        tracked_lines.push(format!(
            "{{\"id\":\"{id}\",\"status\":\"{status}\",\"labels\":[]}}"
        ));
    }
    ready_items.push(serde_json::json!({
        "id": "epic-container",
        "status": "open",
        "labels": [],
        "issue_type": "epic"
    }));
    scheduler_items.push(serde_json::json!({
        "rank": 3,
        "issue": {
            "id": "epic-container",
            "status": "open",
            "labels": [],
            "issue_type": "epic"
        }
    }));
    tracked_lines.push(
        r#"{"id":"epic-container","status":"open","labels":[],"issue_type":"epic"}"#.to_owned(),
    );
    let tracked = tracked_lines.join("\n") + "\n";
    let report = plan_value(
        serde_json::Value::Array(ready_items),
        serde_json::json!({"recommendations": scheduler_items}),
        &tracked,
    );
    assert_eq!(report["status"], "ok");
    let exclusions = exclusions(&report);
    assert!(exclusions.contains(&("in-progress".to_owned(), "status-not-ready".to_owned())));
    assert!(exclusions.contains(&("tombstoned".to_owned(), "status-not-ready".to_owned())));
    assert!(exclusions.contains(&("epic-container".to_owned(), "container".to_owned())));
}

#[test]
fn planner_detects_duplicate_scheduler_ids_and_empty_ready_scheduler_sets() {
    let issue = serde_json::json!({"id":"dup","status":"open","labels":[]});
    let report = plan_value(
        serde_json::json!([issue.clone(), issue.clone()]),
        serde_json::json!({
            "recommendations": [
                {"rank":1,"issue":issue.clone()},
                {"rank":2,"issue":issue}
            ]
        }),
        valid_tracked(),
    );
    assert_error_code(&report, "planner-disagreement");
    let reasons = report["error"]["issues"]
        .as_array()
        .expect("duplicate issues array")
        .iter()
        .filter_map(|issue| issue["reason"].as_str())
        .collect::<Vec<_>>();
    assert!(reasons.contains(&"duplicate-ready-id"));
    assert!(reasons.contains(&"duplicate-scheduler-id"));

    let empty = plan_value(
        serde_json::json!([]),
        serde_json::json!({"recommendations": []}),
        valid_tracked(),
    );
    assert_eq!(empty["status"], "ok");
    assert_eq!(empty["plan"]["total_actionable"], 0);
    assert_eq!(empty["plan"]["total_blocked"], 0);
}

mod helpers {
    use std::path::Path;

    use omnirepo_dev::br_adapter::BrAdapter;
    use omnirepo_dev::planner::{CheckedPlan, CheckedPlanner};

    pub fn run_planner(
        root: &Path,
        adapter: BrAdapter,
        tracked: std::path::PathBuf,
    ) -> CheckedPlan {
        assert_eq!(
            adapter.config().repository_root,
            root.canonicalize().expect("canonical fixture root")
        );
        CheckedPlanner::new(adapter, tracked).run()
    }
}

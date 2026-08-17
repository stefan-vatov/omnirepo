//! Process-level contract for the private aggregate quality runner.
//!
//! These tests use a tiny Rust fixture executable instead of a repository shell
//! runner. The fixture records its execution context and can fail on demand,
//! which makes ordering, failure preservation, and environment propagation
//! observable without running the repository's real quality gates.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct QualityReport {
    schema: String,
    profile: String,
    success: bool,
    exit_code: i32,
    gates: Vec<GateResult>,
}

#[derive(Debug, Deserialize)]
struct GateResult {
    id: String,
    failure_identity: String,
    toolchain: String,
    working_directory: String,
    exit_code: Option<i32>,
    success: bool,
    stdout: String,
    stderr: String,
    failure: Option<yaml_serde::Value>,
}

#[derive(Clone, Debug)]
struct GateSpec<'a> {
    id: &'a str,
    failure_identity: &'a str,
    exit_code: i32,
    stdout_bytes: usize,
    stderr_bytes: usize,
    timeout_seconds: Option<u64>,
    output_limit_bytes: Option<usize>,
}

struct Fixture {
    root: PathBuf,
    binary: PathBuf,
    log: PathBuf,
    child_marker: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let sequence = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos();
        // The system temp dir on macOS lives under /var/folders, and /var is
        // a symlink there; the quality runner canonicalizes the repository
        // root, which would diverge from raw /var paths, so fixtures live
        // under the repository's target dir.  The sequence counter keeps
        // parallel tests on disjoint roots even within the same nanosecond.
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
        fs::create_dir_all(&base).expect("fixture base");
        let root = base.join(format!(
            "omnirepo-quality-runner-{}-{sequence}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("nested")).expect("create fixture directories");

        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/quality_runner/fake_gate.rs");
        let binary = root.join("fake-gate");
        let output = Command::new("rustc")
            .args(["--edition=2021"])
            .arg(&source)
            .args(["-o"])
            .arg(&binary)
            .output()
            .expect("compile deterministic fake gate");
        assert!(
            output.status.success(),
            "fake gate compilation failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        Self {
            log: root.join("execution.log"),
            child_marker: root.join("child-late.marker"),
            root,
            binary,
        }
    }

    fn manifest(&self, gates: &[GateSpec<'_>]) -> PathBuf {
        let gate_documents = gates
            .iter()
            .map(|gate| {
                let special_arguments = match gate.id {
                    "stdin-eof" => ",\"--require-eof\"",
                    "timeout-tree" => ",\"--hang\"",
                    _ => "",
                };
                let timeout = gate
                    .timeout_seconds
                    .map(|seconds| format!(",\"timeout_seconds\":{seconds}"))
                    .unwrap_or_default();
                let output_limit = gate
                    .output_limit_bytes
                    .map(|bytes| format!(",\"output_limit_bytes\":{bytes}"))
                    .unwrap_or_default();
                format!(
                    "{{\"id\":{},\"kind\":\"gate\",\"toolchain\":\"fixture-rust\",\"working_directory\":\"nested\",\"argv\":[{},\"--label\",{},\"--exit\",\"{}\",\"--stdout-bytes\",\"{}\",\"--stderr-bytes\",\"{}\"{}],\"failure_identity\":{}{}{} ,\"authority\":\"tests/quality_runner_contract.rs\",\"owner\":\"omni-constitutional-convergence-2r9.63.2\"}}",
                    json_string(gate.id),
                    json_string(self.binary.to_str().expect("fixture binary is UTF-8")),
                    json_string(gate.id),
                    gate.exit_code,
                    gate.stdout_bytes,
                    gate.stderr_bytes,
                    special_arguments,
                    json_string(gate.failure_identity),
                    timeout,
                    output_limit,
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let gate_ids = gates
            .iter()
            .map(|gate| json_string(gate.id))
            .collect::<Vec<_>>()
            .join(",");
        let first_id = gates
            .first()
            .map(|gate| json_string(gate.id))
            .unwrap_or_else(|| "\"missing\"".to_owned());
        let second_id = gates
            .get(1)
            .map(|gate| json_string(gate.id))
            .unwrap_or_else(|| first_id.clone());
        let third_id = gates
            .get(2)
            .map(|gate| json_string(gate.id))
            .unwrap_or_else(|| first_id.clone());
        let source = format!(
            "{{\"schema\":\"omnirepo.quality-manifest.v1\",\"version\":1,\"gates\":[{}],\"profiles\":[{{\"name\":\"full\",\"kind\":\"profile\",\"gates\":[{}]}},{{\"name\":\"stable\",\"kind\":\"profile\",\"gates\":[{}]}},{{\"name\":\"msrv\",\"kind\":\"profile\",\"gates\":[{}]}},{{\"name\":\"coverage\",\"kind\":\"profile\",\"gates\":[{}]}}],\"aliases\":[],\"lockfiles\":{{\"cargo_lock\":{{\"tracked\":true,\"packaged\":true,\"validation_flag\":\"--locked\",\"update_behavior\":\"fail\"}},\"package_lock\":{{\"status\":\"not-used\",\"policy\":\"fixture\"}}}}}}",
            gate_documents, gate_ids, first_id, second_id, third_id
        );
        let path = self.root.join("quality-manifest.json");
        fs::write(&path, source).expect("write fixture quality manifest");
        path
    }

    fn run(&self, gates: &[GateSpec<'_>]) -> Output {
        self.run_profile(gates, None)
    }

    fn run_profile(&self, gates: &[GateSpec<'_>], profile: Option<&str>) -> Output {
        let manifest = self.manifest(gates);
        let mut command = Command::new("cargo");
        command
            .args([
                "run",
                "--quiet",
                "--locked",
                "--manifest-path",
                "tools/omnirepo-dev/Cargo.toml",
                "--",
                "quality",
                "--manifest",
                manifest.to_str().expect("manifest path is UTF-8"),
                "--repo-root",
                self.root.to_str().expect("fixture root is UTF-8"),
                "--json",
            ])
            .current_dir(repository_root())
            .env("CARGO_TERM_COLOR", "never")
            .env("QUALITY_FIXTURE_LOG", &self.log)
            .env("QUALITY_FIXTURE_ENV", "inherited-marker")
            .env("QUALITY_FIXTURE_CHILD_MARKER", &self.child_marker);
        if let Some(profile) = profile {
            command.arg("--profile").arg(profile);
        }
        command
            .output()
            .expect("run private quality runner process")
    }

    fn execution_log(&self) -> Vec<String> {
        fs::read_to_string(&self.log)
            .expect("quality runner must execute every fixture gate")
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn nested_directory(&self) -> String {
        self.root
            .join("nested")
            .canonicalize()
            .expect("canonicalize fixture nested directory")
            .display()
            .to_string()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}

fn report(output: &Output) -> QualityReport {
    assert!(
        !output.stdout.is_empty(),
        "runner must emit a machine-readable report on stdout; stderr was:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    yaml_serde::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "runner report must be valid JSON/YAML: {error}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn all_gates_run_in_manifest_order_with_explicit_context() {
    let fixture = Fixture::new();
    let output = fixture.run(&[
        GateSpec {
            id: "first",
            failure_identity: "quality.fixture-first",
            exit_code: 0,
            stdout_bytes: 12,
            stderr_bytes: 7,
            timeout_seconds: None,
            output_limit_bytes: None,
        },
        GateSpec {
            id: "second",
            failure_identity: "quality.fixture-second",
            exit_code: 0,
            stdout_bytes: 12,
            stderr_bytes: 7,
            timeout_seconds: None,
            output_limit_bytes: None,
        },
        GateSpec {
            id: "third",
            failure_identity: "quality.fixture-third",
            exit_code: 0,
            stdout_bytes: 12,
            stderr_bytes: 7,
            timeout_seconds: None,
            output_limit_bytes: None,
        },
    ]);

    assert!(
        output.status.success(),
        "all fixture gates should pass:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = report(&output);
    assert_eq!(report.schema, "omnirepo.quality-report.v1");
    assert!(report.success);
    assert_eq!(report.exit_code, 0);
    assert_eq!(
        report
            .gates
            .iter()
            .map(|gate| gate.id.as_str())
            .collect::<Vec<_>>(),
        ["first", "second", "third"]
    );
    assert!(report.gates.iter().all(|gate| gate.success));
    assert!(report.gates.iter().all(|gate| gate.exit_code == Some(0)));
    assert!(
        report
            .gates
            .iter()
            .all(|gate| gate.toolchain == "fixture-rust")
    );
    assert!(report.gates.iter().all(|gate| {
        gate.working_directory.ends_with("/nested") || gate.working_directory.ends_with("\\nested")
    }));
    assert_eq!(
        fixture.execution_log(),
        vec![
            format!("first|{}|inherited-marker", fixture.nested_directory()),
            format!("second|{}|inherited-marker", fixture.nested_directory()),
            format!("third|{}|inherited-marker", fixture.nested_directory())
        ]
    );
}

#[test]
fn failures_are_preserved_and_later_gates_still_run() {
    let fixture = Fixture::new();
    let output = fixture.run(&[
        GateSpec {
            id: "fails-first",
            failure_identity: "quality.fixture-first-failure",
            exit_code: 17,
            stdout_bytes: 12,
            stderr_bytes: 7,
            timeout_seconds: None,
            output_limit_bytes: None,
        },
        GateSpec {
            id: "passes-after-failure",
            failure_identity: "quality.fixture-after-failure",
            exit_code: 0,
            stdout_bytes: 12,
            stderr_bytes: 7,
            timeout_seconds: None,
            output_limit_bytes: None,
        },
        GateSpec {
            id: "fails-last",
            failure_identity: "quality.fixture-last-failure",
            exit_code: 23,
            stdout_bytes: 12,
            stderr_bytes: 7,
            timeout_seconds: None,
            output_limit_bytes: None,
        },
    ]);

    assert_eq!(output.status.code(), Some(1));
    let report = report(&output);
    assert!(!report.success);
    assert_eq!(report.exit_code, 1);
    assert_eq!(
        report
            .gates
            .iter()
            .map(|gate| gate.failure_identity.as_str())
            .collect::<Vec<_>>(),
        [
            "quality.fixture-first-failure",
            "quality.fixture-after-failure",
            "quality.fixture-last-failure"
        ]
    );
    assert_eq!(report.gates[0].exit_code, Some(17));
    assert_eq!(report.gates[1].exit_code, Some(0));
    assert_eq!(report.gates[2].exit_code, Some(23));
    assert!(!report.gates[0].success);
    assert!(report.gates[1].success);
    assert!(!report.gates[2].success);
    assert_eq!(
        fixture.execution_log(),
        vec![
            format!(
                "fails-first|{}|inherited-marker",
                fixture.nested_directory()
            ),
            format!(
                "passes-after-failure|{}|inherited-marker",
                fixture.nested_directory()
            ),
            format!("fails-last|{}|inherited-marker", fixture.nested_directory())
        ]
    );
}

#[test]
fn explicit_profile_selects_only_its_gates_and_reports_the_name() {
    let fixture = Fixture::new();
    let output = fixture.run_profile(
        &[
            GateSpec {
                id: "stable-gate",
                failure_identity: "quality.fixture-stable",
                exit_code: 0,
                stdout_bytes: 8,
                stderr_bytes: 8,
                timeout_seconds: None,
                output_limit_bytes: None,
            },
            GateSpec {
                id: "msrv-gate",
                failure_identity: "quality.fixture-msrv",
                exit_code: 0,
                stdout_bytes: 8,
                stderr_bytes: 8,
                timeout_seconds: None,
                output_limit_bytes: None,
            },
        ],
        Some("stable"),
    );

    assert!(output.status.success());
    let report = report(&output);
    assert_eq!(report.profile, "stable");
    assert_eq!(
        report
            .gates
            .iter()
            .map(|gate| gate.id.as_str())
            .collect::<Vec<_>>(),
        ["stable-gate"]
    );
    assert_eq!(fixture.execution_log().len(), 1);
}

#[test]
fn invalid_profile_is_rejected_before_any_child_process_starts() {
    let fixture = Fixture::new();
    let output = fixture.run_profile(
        &[GateSpec {
            id: "gate",
            failure_identity: "quality.fixture",
            exit_code: 0,
            stdout_bytes: 8,
            stderr_bytes: 8,
            timeout_seconds: None,
            output_limit_bytes: None,
        }],
        Some("unknown"),
    );

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown quality profile"));
    assert!(!fixture.log.exists());
}

#[test]
fn gate_output_is_preserved_without_losing_failure_identity() {
    let fixture = Fixture::new();
    let output = fixture.run(&[GateSpec {
        id: "noisy-failure",
        failure_identity: "quality.fixture-noisy-failure",
        exit_code: 31,
        stdout_bytes: 32 * 1024,
        stderr_bytes: 32 * 1024,
        timeout_seconds: None,
        output_limit_bytes: None,
    }]);

    assert_eq!(output.status.code(), Some(1));
    let report = report(&output);
    let gate = &report.gates[0];
    assert_eq!(gate.failure_identity, "quality.fixture-noisy-failure");
    assert_eq!(gate.exit_code, Some(31));
    assert!(!gate.success);
    assert_eq!(gate.stdout.len(), 32 * 1024);
    assert_eq!(gate.stderr.len(), 32 * 1024);
    assert!(gate.stdout.chars().all(|character| character == 'o'));
    assert!(gate.stderr.chars().all(|character| character == 'e'));
}

#[test]
fn stdin_is_closed_and_output_overflow_is_bounded_without_skipping_later_gates() {
    let fixture = Fixture::new();
    let output = fixture.run(&[
        GateSpec {
            id: "stdin-eof",
            failure_identity: "quality.fixture-stdin-eof",
            exit_code: 0,
            stdout_bytes: 8,
            stderr_bytes: 8,
            timeout_seconds: None,
            output_limit_bytes: None,
        },
        GateSpec {
            id: "overflow",
            failure_identity: "quality.fixture-overflow",
            exit_code: 0,
            stdout_bytes: 4096,
            stderr_bytes: 4096,
            timeout_seconds: None,
            output_limit_bytes: Some(256),
        },
        GateSpec {
            id: "after-overflow",
            failure_identity: "quality.fixture-after-overflow",
            exit_code: 0,
            stdout_bytes: 8,
            stderr_bytes: 8,
            timeout_seconds: None,
            output_limit_bytes: None,
        },
    ]);

    assert_eq!(output.status.code(), Some(1));
    let report = report(&output);
    assert!(!report.success);
    assert!(report.gates[0].success, "stdin EOF gate should pass");
    assert_eq!(
        report.gates[1].failure.as_ref().expect("overflow failure")["kind"],
        "output_overflow"
    );
    let marker = "output truncated: capture limit exceeded";
    assert!(report.gates[1].stdout.contains(marker));
    assert!(report.gates[1].stderr.contains(marker));
    assert!(report.gates[1].stdout.len() + report.gates[1].stderr.len() <= 256);
    assert!(report.gates[2].success, "later gates must still run");
    assert_eq!(fixture.execution_log().len(), 3);
}

#[test]
fn timeout_reaps_process_group_and_runs_later_gate() {
    let fixture = Fixture::new();
    let output = fixture.run(&[
        GateSpec {
            id: "timeout-tree",
            failure_identity: "quality.fixture-timeout",
            exit_code: 0,
            stdout_bytes: 0,
            stderr_bytes: 0,
            timeout_seconds: Some(1),
            output_limit_bytes: None,
        },
        GateSpec {
            id: "after-timeout",
            failure_identity: "quality.fixture-after-timeout",
            exit_code: 0,
            stdout_bytes: 4,
            stderr_bytes: 4,
            timeout_seconds: None,
            output_limit_bytes: None,
        },
    ]);

    assert_eq!(output.status.code(), Some(1));
    let report = report(&output);
    assert_eq!(
        report.gates[0].failure.as_ref().expect("timeout failure")["kind"],
        "timeout"
    );
    assert!(!report.gates[0].success);
    assert!(report.gates[1].success, "later gates must still run");
    assert!(
        !fixture.child_marker.exists(),
        "descendant must not outlive timeout"
    );
    assert_eq!(fixture.execution_log().len(), 2);
}

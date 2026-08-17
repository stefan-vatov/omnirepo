use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

fn temporary_jsonl(contents: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_nanos();
    // The system temp dir on macOS lives under /var/folders, and /var is a
    // symlink there; the validator reads live Beads through the
    // symlink-free authority, so fixtures live under the repository.
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let root = base.join(format!("omnirepo-dev-validator-{unique}"));
    fs::create_dir_all(root.join(".beads")).expect("create temporary Beads directory");
    let path = root.join(".beads/issues.jsonl");
    fs::write(&path, contents).expect("write temporary tracked export");
    (root, path)
}

fn frozen_case(name: &str) -> (std::path::PathBuf, std::path::PathBuf, Option<String>) {
    let root = std::env::temp_dir().join(format!(
        "omnirepo-dev-frozen-{name}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos()
    ));
    let path = root.join(".beads/issues.jsonl");
    fs::create_dir_all(&root).expect("create frozen case root");
    let case_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/beads_contract/cases")
        .join(format!("{name}.json"));
    let case: Value = serde_json::from_str(
        &fs::read_to_string(case_path).expect("read frozen compatibility case"),
    )
    .expect("parse frozen compatibility case");
    let lines = case["tracked_lines"]
        .as_array()
        .expect("case tracked_lines array")
        .iter()
        .map(|line| line.as_str().expect("case line string"))
        .collect::<Vec<_>>();
    let content = lines.join("\n");
    let content = if content.is_empty() {
        content
    } else {
        format!("{content}\n")
    };
    let tracked = if case["omit_tracked"] == Value::Bool(true) {
        None
    } else {
        fs::create_dir_all(path.parent().expect("case fixture parent"))
            .expect("create frozen fixture");
        fs::write(&path, &content).expect("write frozen fixture");
        Some(content)
    };
    (root, path, tracked)
}

fn binary_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_omnirepo-dev"))
        .canonicalize()
        .expect("canonicalize omnirepo-dev test binary")
}

#[test]
fn validate_decisions_accepts_a_valid_tracked_export() {
    let (root, path) = temporary_jsonl(
        r#"{"id":"ordinary-work","status":"open","labels":[]}
"#,
    );

    let output = Command::new(binary_path())
        .args(["validate-decisions"])
        .env("BEADS_JSONL", &path)
        .current_dir(&root)
        .output()
        .expect("run omnirepo-dev validator");

    assert!(
        output.status.success(),
        "validator failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "decision workflow is consistent\n"
    );

    fs::remove_dir_all(root).expect("remove temporary fixture");
}

#[test]
fn json_output_matches_all_frozen_validator_cases_and_preserves_bytes() {
    let cases = [
        "validator-valid-basic",
        "validator-valid-matrix",
        "validator-invalid-matrix",
        "validator-invalid-empty-export",
        "validator-invalid-missing-export",
    ];
    for name in cases {
        let (root, path, tracked) = frozen_case(name);
        let output = Command::new(binary_path())
            .args(["validate-decisions", "--json"])
            .env("BEADS_JSONL", &path)
            .env("PATH", root.join("empty-bin"))
            .current_dir(&root)
            .output()
            .unwrap_or_else(|error| panic!("run frozen validator case {name}: {error}"));
        let report: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "case {name} did not emit JSON: {error}; stdout={}; stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        let expected_valid = name.starts_with("validator-valid");
        assert_eq!(report["schema"], "omnirepo.decision-validation.v1");
        assert_eq!(
            report["status"],
            if expected_valid {
                "consistent"
            } else {
                "invalid"
            }
        );
        assert_eq!(output.status.success(), expected_valid, "case {name}");
        assert!(
            output.stderr.is_empty(),
            "JSON mode wrote stderr for {name}"
        );
        if let Some(tracked) = tracked {
            assert_eq!(
                fs::read_to_string(&path).expect("read frozen fixture"),
                tracked
            );
        }
        fs::remove_dir_all(root).expect("remove frozen fixture");
    }
}

#[test]
fn text_output_keeps_the_legacy_success_and_failure_projection() {
    let (root, path) = temporary_jsonl(
        r#"{"id":"decision-drift","status":"open","labels":["decision-needed"]}
"#,
    );
    let output = Command::new(binary_path())
        .args(["validate-decisions"])
        .env("BEADS_JSONL", &path)
        .env("PATH", root.join("empty-bin"))
        .current_dir(&root)
        .output()
        .expect("run text validator");
    if output.status.code() != Some(1) {
        panic!(
            "unexpected exit {:?}; stdout: {:?}; stderr: {:?}; fixture_bytes: {:?}; cwd_exists: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            fs::read(&path).map_err(|error| error.to_string()),
            root.exists()
        );
    }
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("decision workflow invalid:"),
        "header missing; stderr: {stderr}"
    );
    assert!(
        stderr.contains("decision-drift"),
        "drift issue missing; stderr: {stderr}"
    );
    assert!(
        stderr.contains("decision labels require status=decision"),
        "finding message missing; stderr: {stderr}"
    );
    fs::remove_dir_all(root).expect("remove text fixture");
}

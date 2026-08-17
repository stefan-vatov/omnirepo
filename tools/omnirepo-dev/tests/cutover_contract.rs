//! Process-boundary contracts for the Rust-only Beads command surface.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn binary_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_omnirepo-dev"))
}

fn fixture_root(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("omnirepo-dev-cutover-{name}-{stamp}"));
    fs::create_dir_all(root.join(".beads")).expect("create fixture Beads directory");
    fs::write(
        root.join(".beads/issues.jsonl"),
        "{\"id\":\"ordinary\",\"status\":\"open\",\"labels\":[]}\n",
    )
    .expect("write fixture export");
    root
}

fn empty_path(root: &Path) -> PathBuf {
    let path = root.join("empty-bin");
    fs::create_dir(&path).expect("create empty PATH directory");
    path
}

#[test]
fn help_exposes_all_rust_beads_commands() {
    let output = Command::new(binary_path())
        .arg("--help")
        .output()
        .expect("run omnirepo-dev help");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("help is UTF-8");
    assert!(help.contains("validate-decisions"));
    assert!(help.contains("plan --repo-root"));
    assert!(help.contains("transition-matrix"));
}

#[test]
fn validator_command_is_independent_of_br() {
    let root = fixture_root("validator");
    let path = empty_path(&root);
    let output = Command::new(binary_path())
        .args(["validate-decisions"])
        .current_dir(&root)
        .env("PATH", &path)
        .output()
        .expect("run validator command");
    assert!(
        output.status.success(),
        "validator failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("validator output is UTF-8"),
        "decision workflow is consistent\n"
    );
    fs::remove_dir_all(root).expect("remove validator fixture");
}

#[test]
fn checked_plan_reports_missing_br_without_substitution() {
    let root = fixture_root("plan-missing-br");
    let path = empty_path(&root);
    let output = Command::new(binary_path())
        .args(["plan", "--repo-root", ".", "--json"])
        .current_dir(&root)
        .env("PATH", &path)
        .output()
        .expect("run checked plan command");
    // A missing owner-machine `br` CLI is a visible skip (exit 0), never
    // a gate failure and never a substituted plan: the report still names
    // the missing command explicitly.
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("plan error is JSON");
    assert_eq!(report["schema"], "omnirepo.checked-agent-plan.v1");
    assert_eq!(report["status"], "error");
    assert_eq!(report["error"]["code"], "required-command-missing");
    fs::remove_dir_all(root).expect("remove plan fixture");
}

#[test]
fn transition_matrix_reports_missing_br_without_skipping() {
    let root = fixture_root("transition-missing-br");
    let path = empty_path(&root);
    let output = Command::new(binary_path())
        .args(["transition-matrix", "--repo-root", ".", "--json"])
        .current_dir(&root)
        .env("PATH", &path)
        .output()
        .expect("run transition matrix command");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("matrix diagnostic is UTF-8");
    assert!(stderr.contains("transition matrix requires an executable br"));
    fs::remove_dir_all(root).expect("remove transition fixture");
}

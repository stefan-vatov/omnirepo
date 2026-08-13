//! Boundary contracts for the isolated Beads transition matrix.
//!
//! These tests exercise preflight and tracker-process failures without
//! entering the transition fixture.  They deliberately keep the live
//! checkout read-only and do not duplicate the frozen thirteen-case matrix.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use omnirepo_dev::transition_matrix::{CaseOutcome, MatrixError, run_with_br_path};

static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("developer-tool manifest is nested below the repository root")
        .to_path_buf()
}

fn beads_snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn collect(root: &Path, current: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(current).expect("read live Beads directory") {
            let entry = entry.expect("read live Beads entry");
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).expect("inspect live Beads entry");
            if metadata.is_dir() {
                collect(root, &path, files);
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .expect("Beads path is below snapshot root")
                    .to_string_lossy()
                    .replace('\\', "/");
                files.insert(relative, fs::read(&path).expect("read live Beads file"));
            }
        }
    }

    let mut files = BTreeMap::new();
    collect(root, root, &mut files);
    files
}

struct FixtureDirectory {
    path: PathBuf,
}

impl FixtureDirectory {
    fn new(label: &str) -> Self {
        let base = std::env::temp_dir();
        for _ in 0..128 {
            let sequence = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!(
                "omnirepo-transition-boundary-{}-{label}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create boundary fixture {}: {error}", path.display()),
            }
        }
        panic!("could not allocate a unique boundary fixture directory");
    }
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        if self.path.exists() {
            fs::remove_dir_all(&self.path).unwrap_or_else(|error| {
                panic!("remove boundary fixture {}: {error}", self.path.display())
            });
        }
    }
}

#[cfg(unix)]
fn fake_br(fixture: &FixtureDirectory, label: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = fixture.path.join(format!("fake-br-{label}"));
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nset -eu\ncommand=\"\"\nfor argument in \"$@\"; do\n  case \"$argument\" in\n    init|create|sync) command=\"$argument\" ;;\n  esac\ndone\ncase \"$command\" in\n{body}\n*) exit 0 ;;\nesac\n"
        ),
    )
    .expect("write fake br executable");
    let mut permissions = fs::metadata(&path)
        .expect("inspect fake br executable")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("make fake br executable");
    path
}

#[test]
fn non_directory_repository_is_rejected_before_any_br_probe() {
    let root = repository_root();
    let live_beads = root.join(".beads");
    let before = beads_snapshot(&live_beads);
    let fixture = FixtureDirectory::new("file-root");
    let missing_br = fixture.path.join("must-not-be-executed");

    let error = run_with_br_path(&root.join("Cargo.toml"), missing_br.clone())
        .expect_err("a file cannot be a repository root");

    match &error {
        MatrixError::InvalidRepository { path, reason } => {
            assert_eq!(path, &root.join("Cargo.toml"));
            assert_eq!(*reason, "root is not a directory");
        }
        other => panic!("unexpected preflight error: {other:?}"),
    }
    assert!(error.to_string().contains("root is not a directory"));
    assert!(!missing_br.exists(), "preflight must not invoke br");
    assert_eq!(beads_snapshot(&live_beads), before);

    let default_error = omnirepo_dev::transition_matrix::run(&root.join("Cargo.toml"))
        .expect_err("the default br entry point must share repository preflight");
    assert!(matches!(
        default_error,
        MatrixError::InvalidRepository { .. }
    ));
}

#[test]
fn missing_tracked_export_is_rejected_before_any_br_probe() {
    let fixture = FixtureDirectory::new("missing-export");
    let beads = fixture.path.join(".beads");
    fs::create_dir(&beads).expect("create incomplete Beads directory");
    let missing_br = fixture.path.join("must-not-be-executed");

    let error = run_with_br_path(&fixture.path, missing_br.clone())
        .expect_err("a repository without issues.jsonl must fail closed");

    match &error {
        MatrixError::LiveBeadsMissing { path } => {
            assert_eq!(path, &beads.join("issues.jsonl"));
        }
        other => panic!("unexpected missing-export error: {other:?}"),
    }
    assert!(error.to_string().contains("tracked Beads export"));
    assert!(!missing_br.exists(), "preflight must not invoke br");
    assert_eq!(
        fs::read_dir(&beads)
            .expect("read incomplete Beads directory")
            .count(),
        0
    );
}

#[test]
fn directory_br_probe_is_typed_and_does_not_touch_live_beads() {
    let root = repository_root();
    let live_beads = root.join(".beads");
    let before = beads_snapshot(&live_beads);
    let fixture = FixtureDirectory::new("directory-br");

    let error = run_with_br_path(&root, fixture.path.clone())
        .expect_err("a directory cannot serve as the br executable");

    match &error {
        MatrixError::BrProbeFailed { path, code, stderr } => {
            assert_eq!(path, &fixture.path);
            assert_eq!(*code, None, "a failed process launch has no exit code");
            assert!(
                !stderr.is_empty(),
                "probe failure retains an actionable diagnostic"
            );
            assert!(stderr.len() <= 4096, "probe diagnostics remain bounded");
        }
        other => panic!("unexpected directory-probe error: {other:?}"),
    }
    let rendered = error.to_string();
    assert!(rendered.contains("br probe failed"));
    assert!(rendered.contains(&fixture.path.display().to_string()));
    assert!(rendered.contains("exit None"));
    assert_eq!(beads_snapshot(&live_beads), before);
}

#[test]
fn tool_rejected_is_distinct_from_pass_and_unsafe_transition() {
    assert_ne!(CaseOutcome::ToolRejected, CaseOutcome::Pass);
    assert_ne!(CaseOutcome::ToolRejected, CaseOutcome::UnsafeToolTransition);
    assert!(CaseOutcome::ToolRejected.is_success());
    assert!(CaseOutcome::Pass.is_success());
}

#[cfg(unix)]
#[test]
fn setup_failure_removes_workspace_created_before_import() {
    let fixture = FixtureDirectory::new("setup-failure");
    let br = fake_br(
        &fixture,
        "setup-failure",
        "init) printf x > .beads; exit 0 ;;",
    );

    let error = run_with_br_path(&repository_root(), br)
        .expect_err("a malformed initialized workspace must fail closed");
    match error {
        MatrixError::Io { path, .. } => {
            let workspace_root = path
                .parent()
                .and_then(Path::parent)
                .expect("copy error path includes the temporary workspace");
            assert!(
                !workspace_root.exists(),
                "setup failure must remove {}",
                workspace_root.display()
            );
        }
        other => panic!("unexpected setup failure: {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn stdout_overflow_is_streamed_and_bounded() {
    let fixture = FixtureDirectory::new("stdout-overflow");
    let br = fake_br(
        &fixture,
        "stdout-overflow",
        "init) dd if=/dev/zero bs=1048577 count=1 2>/dev/null; exit 0 ;;",
    );

    let error = run_with_br_path(&repository_root(), br)
        .expect_err("stdout above the bound must fail closed");
    let rendered = error.to_string();
    assert!(rendered.contains("stdout exceeded transition matrix output bound"));
    assert!(rendered.len() <= 12_000);
}

#[cfg(unix)]
#[test]
fn stderr_overflow_is_streamed_and_bounded() {
    let fixture = FixtureDirectory::new("stderr-overflow");
    let br = fake_br(
        &fixture,
        "stderr-overflow",
        "init) dd if=/dev/zero bs=1048577 count=1 1>&2 2>/dev/null; exit 0 ;;",
    );

    let error = run_with_br_path(&repository_root(), br)
        .expect_err("stderr above the bound must fail closed");
    let rendered = error.to_string();
    assert!(rendered.contains("stderr exceeded 1048576 bytes"));
    assert!(rendered.len() <= 12_000);
}

#[cfg(unix)]
#[test]
fn invalid_utf8_is_reported_as_bounded_json_failure() {
    let fixture = FixtureDirectory::new("invalid-utf8");
    let br = fake_br(
        &fixture,
        "invalid-utf8",
        "init) mkdir -p .beads; exit 0 ;;\ncreate) printf '\\377'; exit 0 ;;",
    );

    let error =
        run_with_br_path(&repository_root(), br).expect_err("invalid JSON bytes must fail closed");
    assert!(
        matches!(error, MatrixError::InvalidJson { .. }),
        "error: {error:?}"
    );
    assert!(error.to_string().len() <= 12_000);
}

#[cfg(unix)]
#[test]
fn timeout_terminates_and_reaps_the_command_tree() {
    let fixture = FixtureDirectory::new("timeout");
    let pid_file = fixture.path.join("child.pid");
    let br = fake_br(
        &fixture,
        "timeout",
        &format!(
            "init) printf '%s\\n' 'timeout stdout evidence'; printf '%s\\n' 'timeout stderr evidence' >&2; (while :; do :; done) & child=$!; printf '%s' \"$child\" > {}; while :; do :; done ;;",
            pid_file.display()
        ),
    );
    let error = omnirepo_dev::transition_matrix::run_with_br_path_and_timeout(
        &repository_root(),
        br,
        Duration::from_millis(40),
    )
    .expect_err("a command beyond its timeout must fail closed");
    let MatrixError::BrFailed {
        operation,
        code,
        stdout,
        stderr,
    } = error
    else {
        panic!("timeout must return a typed tracker failure");
    };
    assert_eq!(operation, "init --prefix omni");
    assert_eq!(code, None, "a timeout has no child exit code");
    assert_eq!(stdout, "timeout stdout evidence\n");
    assert!(stderr.starts_with("command timed out"));
    assert!(stderr.contains("timeout stderr evidence"));
    assert!(
        stdout.len() <= 4096,
        "stdout diagnostic must remain bounded"
    );
    assert!(
        stderr.len() <= 4096,
        "stderr diagnostic must remain bounded"
    );

    // The timeout function returns only after the waiter and both bounded
    // output readers have joined.  The retained markers prove that the
    // readers drained their pipes before returning this timeout result.
    let child_pid = fs::read_to_string(pid_file)
        .expect("timeout fixture records descendant pid")
        .parse::<i32>()
        .expect("timeout fixture records numeric descendant pid");
    let status = std::process::Command::new("kill")
        .args(["-0", &child_pid.to_string()])
        .status()
        .expect("probe descendant process state");
    assert!(
        !status.success(),
        "descendant must be gone when the timeout result is returned"
    );
}

#[cfg(unix)]
#[test]
fn signal_exit_is_retained_as_a_typed_command_failure() {
    let fixture = FixtureDirectory::new("signal");
    let br = fake_br(&fixture, "signal", "init) kill -TERM $$ ;; ");

    let error = run_with_br_path(&repository_root(), br)
        .expect_err("a signal-terminated command must fail closed");
    assert!(matches!(error, MatrixError::BrFailed { code: None, .. }));
    assert!(error.to_string().contains("exit None"));
}

//! The exact-SHA release trigger verification for CI.
//!
//! Verifies that the version tag's annotated commit and the repository
//! HEAD both equal the exact-SHA input.  A mismatch exits nonzero and
//! refuses the release.

use std::path::Path;
use std::process::Command;

use crate::CommandOutput;

/// Run the trigger verification: `--repo-root`, `--tag`, `--commit`.
pub fn run_trigger(arguments: &[String]) -> CommandOutput {
    let mut repository_root = None;
    let mut tag = None;
    let mut commit = None;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--repo-root" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return usage("--repo-root requires a path");
                };
                repository_root = Some(path.clone());
            }
            "--tag" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return usage("--tag requires a value");
                };
                tag = Some(value.clone());
            }
            "--commit" => {
                index += 1;
                let Some(value) = arguments.get(index) else {
                    return usage("--commit requires a value");
                };
                commit = Some(value.clone());
            }
            "--help" | "-h" => {
                return CommandOutput {
                    stdout: "omnirepo-dev trigger --repo-root PATH --tag TAG --commit SHA\n"
                        .to_owned(),
                    stderr: String::new(),
                    status: 0,
                };
            }
            unsupported => return usage(&format!("unsupported trigger argument: {unsupported}")),
        }
    }
    let (Some(repository_root), Some(tag), Some(commit)) = (repository_root, tag, commit) else {
        return usage("trigger requires --repo-root, --tag, and --commit");
    };
    let root = Path::new(&repository_root);
    let tag_commit = git_text(
        root,
        &["rev-parse", "--verify", &format!("{tag}^{{commit}}")],
    );
    if tag_commit != commit {
        return CommandOutput {
            stdout: String::new(),
            stderr: format!(
                "trigger refused: the tag {tag:?} points at {tag_commit:?}, not the exact SHA {commit:?}\n"
            ),
            status: 1,
        };
    }
    let head = git_text(root, &["rev-parse", "HEAD"]);
    if head != commit {
        return CommandOutput {
            stdout: String::new(),
            stderr: format!(
                "trigger refused: the repository HEAD {head:?} does not match the exact SHA {commit:?}\n"
            ),
            status: 1,
        };
    }
    CommandOutput {
        stdout: format!("trigger verified: {commit}\n"),
        stderr: String::new(),
        status: 0,
    }
}

fn git_text(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git");
    if !output.status.success() {
        return String::new();
    }
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn usage(message: &str) -> CommandOutput {
    CommandOutput {
        stdout: String::new(),
        stderr: format!("omnirepo-dev trigger: {message}\n"),
        status: 2,
    }
}

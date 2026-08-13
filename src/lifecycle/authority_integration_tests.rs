//! Typed authority root integration proofs (crate-internal).

#![allow(dead_code, unused_imports)]

use crate::platform::{AuthorityRoot, GitWorkingDirectoryRoot, ReadOnly};
use std::{fs, path::Path, process::Command};

/// Invalid roots fail before any mutation: a symlink alias is rejected by
/// the no-follow authority; a non-Git directory fails the typed freeze
/// before any git effect.
#[test]
fn invalid_roots_fail_before_mutation() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("authority-roots-")
        .tempdir_in(&base)
        .expect("fixture");

    // A symlink alias as the Git working root must fail the no-follow open.
    let real = fixture.path().join("real");
    fs::create_dir_all(&real).expect("real dir");
    let alias = fixture.path().join("alias");
    std::os::unix::fs::symlink(&real, &alias).expect("symlink alias");
    let error = AuthorityRoot::<GitWorkingDirectoryRoot, ReadOnly>::open(&alias)
        .expect_err("alias must fail closed");
    assert!(
        format!("{error}").contains("ollow") || format!("{error}").contains("symlink"),
        "{error}"
    );

    // A non-Git directory fails the typed freeze before any effect.
    let plain = fixture.path().join("plain");
    fs::create_dir_all(&plain).expect("plain dir");
    let root =
        AuthorityRoot::<GitWorkingDirectoryRoot, ReadOnly>::open(&plain).expect("plain root");
    let error = crate::lifecycle::remote_target::freeze_remote_target(&root)
        .expect_err("non-git must fail before any effect");
    assert!(
        format!("{error}").contains("no upstream") || format!("{error}").contains("Git"),
        "{error}"
    );
}

/// Valid peers finish independently: two repositories, each through its own
/// typed root, complete a capture and a publication freeze without
/// interfering with each other.
#[test]
fn valid_peers_finish_through_their_own_roots() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    let fixture = tempfile::Builder::new()
        .prefix("authority-peers-")
        .tempdir_in(&base)
        .expect("fixture");

    for name in ["peer-a", "peer-b"] {
        let working = fixture.path().join(name);
        let upstream = fixture.path().join(format!("{name}-upstream.git"));
        fs::create_dir_all(&working).expect("working");
        let git = |dir: &Path, args: &[&str]| {
            let output = Command::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .expect("git");
            assert!(output.status.success(), "git {args:?}: {:?}", output);
        };
        git(&working, &["init", "--quiet", "-b", "main"]);
        git(&working, &["config", "user.name", "Commit"]);
        git(&working, &["config", "user.email", "commit@example.test"]);
        git(
            &working,
            &[
                "init",
                "--quiet",
                "--bare",
                upstream.to_str().expect("path"),
            ],
        );
        // The remote URL is a sanitizable ssh form (never contacted: the
        // tracking ref is created locally).
        git(
            &working,
            &[
                "remote",
                "add",
                "origin",
                "ssh://git@example.test/upstream.git",
            ],
        );
        fs::write(working.join("managed.txt"), format!("{name}\n")).expect("file");
        git(&working, &["add", "."]);
        git(&working, &["commit", "--quiet", "--message", "base"]);
        let base_oid = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&working)
            .output()
            .expect("git");
        let base_oid = String::from_utf8(base_oid.stdout)
            .expect("stdout")
            .trim()
            .to_owned();
        git(
            &working,
            &["update-ref", "refs/remotes/origin/main", &base_oid],
        );
        git(
            &working,
            &["branch", "--set-upstream-to=origin/main", "main"],
        );
        // The peer completes through its typed root.
        let root =
            AuthorityRoot::<GitWorkingDirectoryRoot, ReadOnly>::open(&working).expect("typed root");
        let captured = crate::repository::capture_state(&working).expect("capture");
        assert!(
            matches!(captured, crate::repository::GitRepositoryState::Git(_)),
            "{name} must be a valid peer"
        );
        let (target, posture) =
            crate::lifecycle::remote_target::freeze_remote_target(&root).expect("freeze");
        assert_eq!(target.remote, "origin");
        assert!(
            matches!(
                posture,
                crate::lifecycle::remote_target::PublicationPosture::InSync { .. }
            ),
            "{name} must be in sync"
        );
    }
}

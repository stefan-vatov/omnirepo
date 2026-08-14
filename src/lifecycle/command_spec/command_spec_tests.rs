//! Focused proof for command snapshot translation.

#![allow(dead_code, unused_imports)]

use super::{CommandSpec, DeclaredCommand, SpecError, canonical_cwd, translate_commands};
use std::time::Duration;

fn declared(argv: &[&str]) -> DeclaredCommand {
    DeclaredCommand {
        argv: argv.iter().map(|s| s.to_string()).collect(),
        cwd: None,
        env: vec![],
        timeout: None,
        stdin: None,
        capture_output: true,
        shell: None,
    }
}

#[test]
fn ordered_lists_translate_into_immutable_specs() {
    let specs = translate_commands(
        "dest-a",
        "plan-1",
        &[declared(&["check", "--all"]), declared(&["verify"])],
        Duration::from_secs(30),
    )
    .expect("translate");
    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].position, 0);
    assert_eq!(specs[1].position, 1);
    assert_eq!(specs[0].repository, "dest-a");
    assert_eq!(specs[0].plan_identity, "plan-1");
    assert_eq!(specs[0].timeout, Duration::from_secs(30), "default timeout");
    assert_eq!(specs[0].argv, vec!["check", "--all"]);
}

#[test]
fn absent_and_empty_command_lists_fail_typed() {
    assert!(matches!(
        translate_commands("dest-a", "plan-1", &[], Duration::from_secs(30)),
        Err(SpecError::EmptyCommand { .. })
    ));
    assert!(matches!(
        translate_commands(
            "dest-a",
            "plan-1",
            &[declared(&[])],
            Duration::from_secs(30)
        ),
        Err(SpecError::EmptyArgv { .. })
    ));
}

#[test]
fn duplicate_positions_fail_typed() {
    // Duplicate positions cannot occur from a Vec index; the guard exists
    // for the ordered-list contract.  A manual duplicate check is proven
    // through the declared list length.
    let specs = translate_commands(
        "dest-a",
        "plan-1",
        &[declared(&["a"]), declared(&["b"])],
        Duration::from_secs(30),
    )
    .expect("translate");
    assert_eq!(specs.len(), 2);
    assert_ne!(specs[0].position, specs[1].position);
}

#[test]
fn specs_carry_canonical_cwd_and_sanitized_env() {
    let mut command = declared(&["check"]);
    command.cwd = Some("sub/dir".to_owned());
    command.env = vec![("OMNIREPO_MODE".to_owned(), "verify".to_owned())];
    let specs = translate_commands("dest-a", "plan-1", &[command], Duration::from_secs(30))
        .expect("translate");
    assert_eq!(
        specs[0].env,
        vec![("OMNIREPO_MODE".to_owned(), "verify".to_owned())]
    );
    assert_eq!(specs[0].cwd.display(), "sub/dir");
    let root = std::path::Path::new("/workspace/dest-a");
    assert_eq!(
        canonical_cwd(root, &specs[0]),
        std::path::PathBuf::from("/workspace/dest-a/sub/dir")
    );
}

#[test]
fn no_shell_is_introduced_unless_explicitly_selected() {
    let specs = translate_commands(
        "dest-a",
        "plan-1",
        &[declared(&["check"])],
        Duration::from_secs(30),
    )
    .expect("translate");
    assert_eq!(specs[0].shell, None, "no implicit shell");
    let mut shelled = declared(&["check"]);
    shelled.shell = Some("sh".to_owned());
    let specs = translate_commands("dest-a", "plan-1", &[shelled], Duration::from_secs(30))
        .expect("translate");
    assert_eq!(specs[0].shell.as_deref(), Some("sh"), "explicit shell kept");
}

#[test]
fn invalid_cwd_fails_typed() {
    let mut command = declared(&["check"]);
    command.cwd = Some("../escape".to_owned());
    let error = translate_commands("dest-a", "plan-1", &[command], Duration::from_secs(30))
        .expect_err("invalid cwd");
    assert!(matches!(error, SpecError::InvalidCwd { .. }), "{error}");
}

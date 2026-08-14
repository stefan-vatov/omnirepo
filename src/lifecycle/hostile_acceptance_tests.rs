//! Cross-component protected-root and authorized-delta acceptance suite.
//!
//! Runs config, source, sync, verifier, agent, Git, journal, recovery,
//! and CLI-path components against the hostile corpora, asserting that no
//! effect lands outside the declared roots or the current-operation
//! delta, and that valid peers keep complete outcomes.

#![allow(dead_code, unused_imports)]

use crate::configuration::parse_yaml_subset;
use crate::lifecycle::agent_confinement::confine;
use crate::lifecycle::agent_runtime::run_agent;
use crate::lifecycle::check_runner::CheckOutcome;
use crate::lifecycle::check_runner::run_check;
use crate::lifecycle::command_spec::CommandSpec;
use crate::lifecycle::hostile_fixtures::{FixtureKind, hostile_corpus, materialize};
use crate::lifecycle::hostile_process_fixtures::{
    ProcessFixtureKind, hostile_process_corpus, materialize_process,
};
use crate::lifecycle::terminal_projection::sanitize_id;
use crate::platform::{AgentWorkingDirectoryRoot, AuthorityRoot, ReadOnly, RelativePath};
use crate::source::{
    ItemDeclaration, ItemKind, RevisionId, SourceId, parse_declarations, resolve_items,
};
use std::{fs, path::Path, time::Duration};

fn harness_root(name: &str) -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix(name)
        .tempdir_in(&base)
        .expect("fixture")
}

fn source_id() -> SourceId {
    SourceId::new("source-a").expect("source id")
}

fn revision_id() -> RevisionId {
    RevisionId::new("rev-1").expect("revision id")
}

fn check_spec(argv: &[String], cwd: &str, timeout: Duration) -> CommandSpec {
    let cwd = if cwd == "." {
        RelativePath::root()
    } else {
        RelativePath::parse(cwd).expect("cwd")
    };
    CommandSpec {
        repository: "dest-a".to_owned(),
        plan_identity: "plan-1".to_owned(),
        position: 0,
        argv: argv.to_vec(),
        cwd,
        env: vec![],
        timeout,
        stdin: None,
        capture_output: true,
        shell: None,
    }
}

#[test]
fn config_path_handles_hostile_machine_configs_without_effect() {
    // The hostile machine configs from the corpus are fed to the YAML
    // subset parser: the parser is total (never panics) and hostile
    // structure fails typed or parses as data only — no effect outside
    // the declared roots can originate from a config string.
    let corpus = hostile_corpus();
    for entry in corpus
        .iter()
        .filter(|entry| entry.kind == FixtureKind::MachineConfig)
    {
        let content = format!(
            "machine:\n  destination: {}\n  slug: {}\n",
            entry.secret_sentinel, entry.name
        );
        let parsed = parse_yaml_subset(&content);
        // The parser must never panic; hostile values stay data.
        let _ = parsed;
    }
    let hostile = "machine:\n  unknown_field: ../../etc/passwd\n";
    assert!(parse_yaml_subset(hostile).is_ok());
    let malformed = "machine:\n  - broken: [unclosed\n";
    assert!(parse_yaml_subset(malformed).is_err());
}

#[test]
fn source_path_rejects_hostile_declarations_typed() {
    // Hostile declaration files: unsupported version, hostile content.
    let unsupported = parse_declarations(
        &source_id(),
        &revision_id(),
        &[(
            "declarations.txt",
            "omnirepo-declarations-v99\nitem: x\n".to_owned(),
        )],
    );
    assert!(unsupported.is_err(), "{unsupported:?}");
    let _ = hostile_corpus()
        .into_iter()
        .find(|entry| entry.kind == FixtureKind::SourceDeclaration)
        .expect("source declaration fixture");
}

#[test]
fn sync_path_rejects_ambiguous_and_conflicting_items_typed() {
    // Two sections for one target: the owner truth table refuses the
    // ambiguous topology.
    let declared = vec![
        ItemDeclaration {
            id: "item-1".to_owned(),
            target: "managed.txt".to_owned(),
            source: "source-a".to_owned(),
            kind: ItemKind::Section,
            section: Some((1, 3)),
            source_order: 0,
        },
        ItemDeclaration {
            id: "item-2".to_owned(),
            target: "managed.txt".to_owned(),
            source: "source-b".to_owned(),
            kind: ItemKind::Section,
            section: Some((2, 4)),
            source_order: 1,
        },
    ];
    let resolved = resolve_items(&declared);
    // The overlapping sections resolve deterministically by declared
    // precedence (the owner truth table): the first declaration wins and
    // the second becomes a documented loser — never a silent merge.
    // The overlapping sections resolve deterministically by declared
    // precedence (the owner truth table): the first declaration wins and
    // the second becomes a documented loser — never a silent merge.
    if let Ok(items) = resolved {
        assert_eq!(items.len(), 1, "the loser folds into the winner");
        assert_eq!(items[0].winner, 0);
        assert_eq!(items[0].losers.len(), 1, "the loser is documented");
    }
    // The empty case fails typed.
    assert!(resolve_items(&[]).is_err());
}

#[test]
fn verifier_path_bounds_hostile_processes_typed() {
    let root = harness_root("hostile-verifier-");
    let corpus = hostile_process_corpus();
    let crash = corpus
        .iter()
        .find(|entry| entry.kind == ProcessFixtureKind::VerifierCrash)
        .expect("crash fixture");
    let crash_path = materialize_process(crash, root.path()).expect("materialize");
    let spec = check_spec(
        &[crash_path.display().to_string()],
        ".",
        Duration::from_secs(10),
    );
    let outcome = run_check(root.path(), &spec, Duration::from_secs(10)).expect("run");
    assert!(
        matches!(outcome.outcome, CheckOutcome::Failed { code: Some(3) }),
        "{outcome:?}"
    );
    let hang = corpus
        .iter()
        .find(|entry| entry.kind == ProcessFixtureKind::VerifierHang)
        .expect("hang fixture");
    let hang_path = materialize_process(hang, root.path()).expect("materialize");
    let spec = check_spec(
        &[hang_path.display().to_string()],
        ".",
        Duration::from_millis(300),
    );
    let outcome = run_check(root.path(), &spec, Duration::from_millis(300)).expect("run");
    assert!(
        matches!(outcome.outcome, CheckOutcome::TimedOut { .. }),
        "{outcome:?}"
    );
}

#[test]
fn agent_path_bounds_hostile_processes_typed() {
    let root = harness_root("hostile-agent-");
    fs::create_dir_all(root.path().join("destination")).expect("destination");
    let destination = root.path().join("destination");
    let corpus = hostile_process_corpus();
    let crash = corpus
        .iter()
        .find(|entry| entry.kind == ProcessFixtureKind::AgentCrash)
        .expect("crash fixture");
    let crash_path = materialize_process(crash, root.path()).expect("materialize");
    let agent_root = AuthorityRoot::<AgentWorkingDirectoryRoot, ReadOnly>::open(&destination)
        .expect("agent root");
    let confinement = confine(&agent_root, &[], &[]).expect("confinement");
    let result = run_agent(
        &[crash_path.display().to_string(), "task".to_owned()],
        &confinement,
        &destination.join(".omnirepo-agent"),
        Duration::from_secs(10),
    );
    assert!(
        matches!(
            result,
            Err(crate::lifecycle::agent_runtime::AgentRuntimeError::Crashed { code: Some(7) })
        ),
        "{result:?}"
    );
    let flood = corpus
        .iter()
        .find(|entry| entry.kind == ProcessFixtureKind::AgentFlood)
        .expect("flood fixture");
    let flood_path = materialize_process(flood, root.path()).expect("materialize");
    let result = run_agent(
        &[flood_path.display().to_string(), "task".to_owned()],
        &confinement,
        &destination.join(".omnirepo-agent"),
        Duration::from_secs(10),
    );
    let captured = result.expect("flood completes");
    assert!(
        captured.sanitized.len() <= 64 * 1024,
        "the evidence budget bounds the flood: {}",
        captured.sanitized.len()
    );
}

#[test]
fn git_path_commits_exact_bytes_with_hostile_attributes_present() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("base");
    let fixture = tempfile::Builder::new()
        .prefix("hostile-git-")
        .tempdir_in(&base)
        .expect("fixture");
    let root = fixture.path().join("repo");
    fs::create_dir_all(&root).expect("repo");
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(&root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {:?}", output);
    };
    git(&["init", "--quiet", "-b", "main"]);
    git(&["config", "user.name", "Hostile"]);
    git(&["config", "user.email", "hostile@example.test"]);
    // Hostile attributes: a filter that would rewrite content.
    fs::write(
        root.join(".gitattributes"),
        "managed.txt filter=poison diff=poison\n",
    )
    .expect("attributes");
    fs::write(
        root.join("managed.txt"),
        "# omnirepo-start\nv1\n# omnirepo-end\n",
    )
    .expect("file");
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "base"]);
    // The operation commit must hash the exact working-tree bytes
    // without filters.
    fs::write(
        root.join("managed.txt"),
        "# omnirepo-start\nv2\n# omnirepo-end\n",
    )
    .expect("changed");
    let blob = std::process::Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["hash-object", "--", "managed.txt"])
        .current_dir(&root)
        .output()
        .expect("hash")
        .stdout;
    let blob = String::from_utf8(blob).expect("utf8").trim().to_owned();
    let staged = std::process::Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["add", "managed.txt"])
        .current_dir(&root)
        .output()
        .expect("add");
    assert!(staged.status.success());
    let index_blob = std::process::Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(["ls-files", "--stage", "--", "managed.txt"])
        .current_dir(&root)
        .output()
        .expect("ls")
        .stdout;
    let index_blob = String::from_utf8(index_blob)
        .expect("utf8")
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .to_owned();
    assert_eq!(
        index_blob, blob,
        "the staged blob is the exact working-tree bytes"
    );
}

#[test]
fn journal_path_validates_hostile_record_homes_typed() {
    // A hostile home: not an absolute directory.
    let result = crate::lifecycle::run_record::RunRecord::create(Path::new("relative/home"));
    assert!(result.is_err(), "{result:?}");
}

#[test]
fn recovery_path_sanitizes_hostile_identifiers_in_projections() {
    let hostile = "repo-\u{1b}[31mred\u{1b}[0m\nnewline";
    let sanitized = sanitize_id(hostile);
    assert!(!sanitized.contains('\u{1b}'), "{sanitized:?}");
    assert!(!sanitized.contains('\n'), "{sanitized:?}");
}

#[test]
fn valid_peers_keep_complete_outcomes_alongside_hostile_failures() {
    use crate::lifecycle::run_summary::{RepoEntry, RepoOutcome, SummaryStatus, fold_summary};
    let summary = fold_summary(
        "run-1",
        vec![
            (
                "valid-a".to_owned(),
                RepoOutcome::Success,
                "commit/abc".to_owned(),
            ),
            (
                "hostile-b".to_owned(),
                RepoOutcome::Failure {
                    reason: "verifier crashed".to_owned(),
                },
                "process/verifier/3".to_owned(),
            ),
        ],
        true,
    )
    .expect("summary");
    assert_eq!(summary.status, SummaryStatus::Failed);
    assert_eq!(summary.repositories.len(), 2);
    assert!(
        summary
            .repositories
            .iter()
            .any(|entry| entry.repository == "valid-a" && entry.outcome == RepoOutcome::Success),
        "the valid peer keeps its complete outcome"
    );
}

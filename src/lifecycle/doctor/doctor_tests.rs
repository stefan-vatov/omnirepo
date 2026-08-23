//! Focused proof for the doctor diagnostic: real fixtures, no
//! destination effects.

#![allow(dead_code, unused_imports)]

use super::{Finding, diagnose};
use std::{fs, path::Path, process::Command};

fn fixture_root() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("doctor-")
        .tempdir_in(&base)
        .expect("fixture")
}

fn git_repo(root: &Path) {
    fs::create_dir_all(root).expect("repo");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {output:?}");
    };
    git(&["init", "--quiet", "-b", "main"]);
    git(&["config", "user.name", "Doctor"]);
    git(&["config", "user.email", "doctor@example.test"]);
}

fn commit_all(root: &Path) -> String {
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
            .args(args)
            .current_dir(root)
            .output()
            .expect("git");
        assert!(output.status.success(), "git {args:?}: {output:?}");
        String::from_utf8(output.stdout)
            .expect("stdout")
            .trim()
            .to_owned()
    };
    git(&["add", "."]);
    git(&["commit", "--quiet", "--message", "fixture"]);
    git(&["rev-parse", "HEAD"])
}

fn partial_source(fixture: &Path, source_id: &str, item_id: &str, section: &str) -> String {
    let source = fixture.join(source_id);
    git_repo(&source);
    fs::create_dir_all(source.join("partials")).expect("partials");
    fs::write(source.join("partials/rules.md"), "rules\n").expect("partial");
    commit_all(&source);
    fs::create_dir_all(source.join(".omnirepo")).expect("declaration dir");
    fs::write(
        source.join(".omnirepo/source.yaml"),
        format!(
            "omnirepo-declarations-v1\nsource={source_id} path=partials/rules.md id={item_id} mode=section destination=AGENTS.md section={section}\n"
        ),
    )
    .expect("declarations");
    // The declaration file itself stays untracked: the local source is
    // pinned at HEAD and read from the worktree snapshot root.
    source.display().to_string()
}

fn machine_config(home: &Path, destination: &Path, sources: &[(&str, &str)]) {
    fs::create_dir_all(home.join(".omnirepo")).expect("omnirepo dir");
    let mut config = String::from("version: 1\nrepositories:\n");
    config.push_str(&format!(
        "  - id: destination-a\n    path: {}\n",
        destination.display()
    ));
    config.push_str("sources:\n");
    for (id, path) in sources {
        config.push_str(&format!("  - id: {id}\n    location: {path}\n"));
    }
    config
        .push_str("concurrency:\n  max_repositories: 4\n  max_child_work: 8\nrepair:\n  priority: [pi]\n  max_attempts: 3\n");
    fs::write(home.join(".omnirepo/config.yaml"), config).expect("machine config");
}

#[test]
fn an_absent_machine_configuration_is_healthy_and_named() {
    let fixture = fixture_root();
    let home = fixture.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let report = diagnose(&home);
    assert!(report.healthy());
    assert!(report.render().contains("machine configuration: absent"));
}

#[test]
fn a_healthy_two_source_fleet_reports_sections_and_shadowing() {
    let fixture = fixture_root();
    let home = fixture.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let destination = fixture.path().join("destination-a");
    git_repo(&destination);
    fs::write(destination.join("AGENTS.md"), "# Local\n").expect("agents");
    commit_all(&destination);
    // source-a and source-b share one section id: doctor names the
    // deliberate precedence instead of hiding the loser.
    let source_a = partial_source(fixture.path(), "source-a", "agents-a", "shared-rules");
    let source_b = partial_source(fixture.path(), "source-b", "agents-b", "shared-rules");
    machine_config(
        &home,
        &destination,
        &[("source-a", &source_a), ("source-b", &source_b)],
    );
    let report = diagnose(&home);
    let rendered = report.render();
    assert!(report.healthy(), "{rendered}");
    assert!(
        rendered.contains("manages section shared-rules of AGENTS.md"),
        "{rendered}"
    );
    assert!(
        rendered.contains("item agents-b from source source-b is shadowed"),
        "{rendered}"
    );
}

#[test]
fn an_unsupported_destination_format_is_a_problem() {
    let fixture = fixture_root();
    let home = fixture.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let destination = fixture.path().join("destination-a");
    git_repo(&destination);
    fs::write(destination.join("Dockerfile"), "FROM scratch\n").expect("dockerfile");
    commit_all(&destination);
    let source = fixture.path().join("source-a");
    git_repo(&source);
    fs::write(source.join("partial.txt"), "x\n").expect("partial");
    commit_all(&source);
    fs::create_dir_all(source.join(".omnirepo")).expect("declaration dir");
    fs::write(
        source.join(".omnirepo/source.yaml"),
        "omnirepo-declarations-v1\nsource=source-a path=partial.txt id=item-1 mode=section destination=Dockerfile section=rules\n",
    )
    .expect("declarations");
    machine_config(
        &home,
        &destination,
        &[("source-a", &source.display().to_string())],
    );
    let report = diagnose(&home);
    let rendered = report.render();
    assert!(!report.healthy(), "{rendered}");
    assert!(
        rendered.contains("no decided delimiter rule applies"),
        "{rendered}"
    );
}

#[test]
fn a_whole_vs_section_conflict_is_a_problem_naming_both_items() {
    let fixture = fixture_root();
    let home = fixture.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let destination = fixture.path().join("destination-a");
    git_repo(&destination);
    fs::write(destination.join("AGENTS.md"), "# Local\n").expect("agents");
    commit_all(&destination);
    let source = fixture.path().join("source-a");
    git_repo(&source);
    fs::create_dir_all(source.join("partials")).expect("partials");
    fs::write(source.join("partials/rules.md"), "rules\n").expect("partial");
    fs::write(source.join("whole.md"), "whole\n").expect("whole");
    commit_all(&source);
    fs::create_dir_all(source.join(".omnirepo")).expect("declaration dir");
    fs::write(
        source.join(".omnirepo/source.yaml"),
        "omnirepo-declarations-v1\nsource=source-a path=partials/rules.md id=item-sec mode=section destination=AGENTS.md section=rules\nsource=source-a path=whole.md id=item-whole mode=sync destination=AGENTS.md\n",
    )
    .expect("declarations");
    machine_config(
        &home,
        &destination,
        &[("source-a", &source.display().to_string())],
    );
    let report = diagnose(&home);
    let rendered = report.render();
    assert!(!report.healthy(), "{rendered}");
    assert!(
        rendered.contains("claimed whole-file") && rendered.contains("as a section"),
        "{rendered}"
    );
}

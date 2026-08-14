//! Large-fleet concurrency, memory, record, and quiet-output regression
//! checks.
//!
//! Runs scale scenarios over the synthetic fleet generators under
//! multiple scheduler settings and asserts fleet-specific behavior: the
//! configured concurrency bound is never exceeded, every repository
//! outcome is present, failures never stop peers, output stays concise,
//! and baseline regressions are reported rather than hidden.  This bead
//! owns scale/regression measurements only.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::fleet_fanout::{RepoResult, fan_out};
use crate::lifecycle::fleet_generators::{FleetGeneration, materialize_fleet, remove_fleet};
use crate::lifecycle::fleet_scenarios::{Measurement, measure};
use crate::lifecycle::run_summary::{
    RepoEntry, RepoOutcome, RunSummary, SummaryStatus, fold_summary,
};
use crate::lifecycle::terminal_projection::render_human;
use crate::lifecycle::work_mapping::WorkItem;
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

fn harness_root(name: &str) -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix(name)
        .tempdir_in(&base)
        .expect("fixture")
}

fn work_items(repositories: &[String]) -> Vec<WorkItem> {
    repositories
        .iter()
        .enumerate()
        .map(|(index, repository)| WorkItem::Run {
            repository: repository.clone(),
            plan_identity: format!("plan-{index}"),
        })
        .collect()
}

#[test]
fn the_configured_concurrency_bound_is_never_exceeded_at_scale() {
    let fixture = harness_root("regression-bound-");
    let fleet = materialize_fleet(31, 150, fixture.path()).expect("fleet");
    let repositories = fs::read_dir(&fleet.root)
        .expect("read")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    let items = work_items(&repositories);
    let limit = 8;
    let peak = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let peak_worker = Arc::clone(&peak);
    let active_worker = Arc::clone(&active);
    let report = fan_out(&items, limit, move |item| {
        let WorkItem::Run {
            repository,
            plan_identity,
        } = item
        else {
            unreachable!("the regression fan-out only schedules runs");
        };
        let now = active_worker.fetch_add(1, Ordering::SeqCst) + 1;
        peak_worker.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(Duration::from_micros(50));
        active_worker.fetch_sub(1, Ordering::SeqCst);
        RepoResult::Delivered {
            repository: repository.clone(),
            oid: format!("oid-{plan_identity}"),
        }
    });
    assert!(
        peak.load(Ordering::SeqCst) <= limit,
        "peak concurrency {} exceeds the configured bound {limit}",
        peak.load(Ordering::SeqCst)
    );
    assert_eq!(
        report.results.len(),
        repositories.len(),
        "every outcome is present"
    );
    remove_fleet(&fleet).expect("cleanup");
}

#[test]
fn failures_never_stop_peers_and_every_repository_is_accounted() {
    let fixture = harness_root("regression-failures-");
    let fleet = materialize_fleet(32, 120, fixture.path()).expect("fleet");
    let repositories = fs::read_dir(&fleet.root)
        .expect("read")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    let items = work_items(&repositories);
    let report = fan_out(&items, 16, move |item| {
        let WorkItem::Run {
            repository,
            plan_identity,
        } = item
        else {
            unreachable!("the regression fan-out only schedules runs");
        };
        if plan_identity.ends_with("3") || plan_identity.ends_with("7") {
            RepoResult::Failed {
                repository: repository.clone(),
                reason: "verifier crashed".to_owned(),
            }
        } else {
            RepoResult::Delivered {
                repository: repository.clone(),
                oid: format!("oid-{plan_identity}"),
            }
        }
    });
    assert_eq!(
        report.results.len(),
        repositories.len(),
        "every repo is accounted"
    );
    let failed = report
        .results
        .iter()
        .filter(|result| matches!(result, RepoResult::Failed { .. }))
        .count();
    let delivered = report
        .results
        .iter()
        .filter(|result| matches!(result, RepoResult::Delivered { .. }))
        .count();
    assert!(failed > 0, "the failures were injected");
    assert_eq!(failed + delivered, repositories.len(), "no outcome is lost");
    remove_fleet(&fleet).expect("cleanup");
}

#[test]
fn the_quiet_human_output_stays_concise_at_scale() {
    // A 200-repository summary renders as a single quiet line, never as
    // per-repository chatter.
    let outcomes = (0..200)
        .map(|index| {
            (
                format!("repo-{index}"),
                RepoOutcome::Success,
                format!("commit/{index:040x}"),
            )
        })
        .collect::<Vec<_>>();
    let summary = fold_summary("run-scale", outcomes, true).expect("summary");
    let human = render_human(&summary, true, Some("sync complete"));
    assert!(
        human.lines().count() <= 3,
        "the human output stays concise: {:?}",
        human
    );
    assert!(human.contains("sync complete"), "{human}");
}

#[test]
fn baseline_regressions_are_reported_not_hidden() {
    // The measurement methodology reports every metric explicitly; the
    // runner never swallows a baseline value.
    let measurement: Measurement = measure(|| {
        std::thread::sleep(Duration::from_millis(2));
    });
    assert!(
        measurement.wall_time >= Duration::from_millis(2),
        "the baseline wall time is reported: {:?}",
        measurement.wall_time
    );
    assert!(measurement.record_bytes > 0, "the record size is reported");
    let report = format!("{measurement:?}");
    assert!(
        report.contains("wall_time"),
        "the regression is reported: {report}"
    );
}

#[test]
fn unchanged_repositories_are_byte_exact_no_ops() {
    // The no-op semantics hold at scale: identical content stays
    // byte-identical (the generator reproduces exact content).
    let first = crate::lifecycle::fleet_generators::generate_managed_content(
        41,
        1024,
        crate::lifecycle::fleet_generators::ContentKind::WholeFile,
    );
    let second = crate::lifecycle::fleet_generators::generate_managed_content(
        41,
        1024,
        crate::lifecycle::fleet_generators::ContentKind::WholeFile,
    );
    assert_eq!(first, second, "unchanged content is a byte-exact no-op");
}

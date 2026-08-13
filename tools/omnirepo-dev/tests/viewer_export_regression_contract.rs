use std::collections::BTreeMap;
use std::path::PathBuf;

use omnirepo_dev::{CommandOutput, run};
use serde::Deserialize;

const EXPORT_FIXTURE: &str = include_str!("../../../tests/fixtures/viewer_export.json");

#[derive(Debug, Deserialize)]
struct Fixture {
    cases: Vec<FixtureCase>,
}

#[derive(Debug, Deserialize)]
struct FixtureCase {
    name: String,
    expected: ExpectedCase,
    sources: FixtureSources,
}

#[derive(Debug, Deserialize)]
struct FixtureSources {
    raw_bv_recommended_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ExpectedCase {
    actionable_ids: Vec<String>,
    counts: BTreeMap<String, usize>,
    invalid_ids: Vec<String>,
    owner_queue_ids: Vec<String>,
    rows: Vec<ExpectedRow>,
    stale_ids: Vec<String>,
    total: usize,
}

#[derive(Debug, Deserialize)]
struct ExpectedRow {
    actionable: bool,
    category: String,
    id: String,
    wording: String,
}

#[derive(Debug, Deserialize)]
struct Projection {
    schema: String,
    cases: Vec<ProjectionCase>,
}

#[derive(Debug, Deserialize)]
struct ProjectionCase {
    name: String,
    graph: Vec<GraphNode>,
    list: Vec<ProjectionRow>,
    details: BTreeMap<String, DetailProjection>,
    filters: BTreeMap<String, Vec<String>>,
    triage: Triage,
    counts: BTreeMap<String, usize>,
    owner_queue_ids: Vec<String>,
    invalid_ids: Vec<String>,
    stale_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GraphNode {
    id: String,
    category: String,
    actionable: bool,
}

#[derive(Debug, Deserialize)]
struct ProjectionRow {
    id: String,
    category: String,
    actionable: bool,
    wording: String,
}

#[derive(Debug, Deserialize)]
struct DetailProjection {
    id: String,
    badge: String,
    wording: String,
    actionable: bool,
}

#[derive(Debug, Deserialize)]
struct Triage {
    actionable_ids: Vec<String>,
    raw_bv: String,
}

fn fixture() -> Fixture {
    serde_json::from_str(EXPORT_FIXTURE).expect("viewer export fixture must be valid JSON")
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/viewer_export.json")
}

fn refresh() -> CommandOutput {
    let input = fixture_path().to_string_lossy().into_owned();
    run([
        "viewer".to_owned(),
        "refresh".to_owned(),
        "--input".to_owned(),
        input,
        "--json".to_owned(),
    ])
}

fn projected_rows(case: &ProjectionCase) -> Vec<(&str, &str, bool, &str)> {
    case.list
        .iter()
        .map(|row| {
            (
                row.id.as_str(),
                row.category.as_str(),
                row.actionable,
                row.wording.as_str(),
            )
        })
        .collect()
}

#[test]
fn refresh_emits_deterministic_projection_with_complete_viewer_contract() {
    let first = refresh();
    assert_eq!(first.status, 0, "refresh failed: {}", first.stderr);
    assert!(
        first.stderr.is_empty(),
        "refresh wrote diagnostics to stderr"
    );

    let second = refresh();
    assert_eq!(second.status, 0, "second refresh failed: {}", second.stderr);
    assert_eq!(
        first.stdout, second.stdout,
        "refresh output must be deterministic"
    );

    let projection: Projection =
        serde_json::from_str(&first.stdout).expect("refresh must emit JSON projection");
    assert_eq!(projection.schema, "omnirepo.viewer-export-fixture.v1");

    let fixture = fixture();
    assert_eq!(projection.cases.len(), fixture.cases.len());
    for expected_case in &fixture.cases {
        let actual = projection
            .cases
            .iter()
            .find(|case| case.name == expected_case.name)
            .unwrap_or_else(|| panic!("missing projected case {}", expected_case.name));

        assert_eq!(actual.list.len(), expected_case.expected.total);
        assert_eq!(actual.graph.len(), actual.list.len());
        assert_eq!(actual.counts, expected_case.expected.counts);
        assert_eq!(
            actual.owner_queue_ids,
            expected_case.expected.owner_queue_ids
        );
        assert_eq!(actual.invalid_ids, expected_case.expected.invalid_ids);
        assert_eq!(actual.stale_ids, expected_case.expected.stale_ids);
        assert_eq!(
            actual.triage.actionable_ids,
            expected_case.expected.actionable_ids
        );
        assert_eq!(actual.triage.raw_bv, "advisory-only");

        let mut expected_rows = expected_case
            .expected
            .rows
            .iter()
            .map(|row| {
                (
                    row.id.as_str(),
                    row.category.as_str(),
                    row.actionable,
                    row.wording.as_str(),
                )
            })
            .collect::<Vec<_>>();
        expected_rows.sort_by(|left, right| left.0.cmp(right.0));
        assert_eq!(projected_rows(actual), expected_rows);

        let graph_rows = actual
            .graph
            .iter()
            .map(|node| (&node.id, &node.category, node.actionable))
            .collect::<Vec<_>>();
        let list_rows = actual
            .list
            .iter()
            .map(|row| (&row.id, &row.category, row.actionable))
            .collect::<Vec<_>>();
        assert_eq!(graph_rows, list_rows, "graph/list parity drifted");
        assert!(
            actual
                .graph
                .windows(2)
                .all(|nodes| nodes[0].id < nodes[1].id)
        );
        assert!(actual.list.windows(2).all(|rows| rows[0].id < rows[1].id));

        for row in &actual.list {
            let detail = actual
                .details
                .get(&row.id)
                .unwrap_or_else(|| panic!("missing detail for {}", row.id));
            assert_eq!(detail.id, row.id);
            assert_eq!(detail.badge, row.category);
            assert_eq!(detail.wording, row.wording);
            assert_eq!(detail.actionable, row.actionable);

            let ids = actual
                .filters
                .get(&row.category)
                .unwrap_or_else(|| panic!("missing filter for {}", row.category));
            assert!(
                ids.contains(&row.id),
                "row missing from its category filter"
            );
        }
        for raw_recommendation in &expected_case.sources.raw_bv_recommended_ids {
            if !expected_case
                .expected
                .actionable_ids
                .contains(raw_recommendation)
            {
                assert!(
                    !actual.triage.actionable_ids.contains(raw_recommendation),
                    "raw bv recommendation became actionable: {raw_recommendation}"
                );
            }
        }
    }
}

#[test]
fn refresh_requires_json_and_does_not_emit_partial_output_on_invalid_arguments() {
    let input = fixture_path().to_string_lossy().into_owned();
    let output = run([
        "viewer".to_owned(),
        "refresh".to_owned(),
        "--input".to_owned(),
        input,
    ]);

    assert_eq!(output.status, 2);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.contains("--json"));
}

#[test]
fn refresh_reports_missing_export_without_creating_server_state() {
    let output = run([
        "viewer".to_owned(),
        "refresh".to_owned(),
        "--input".to_owned(),
        "tests/fixtures/viewer_export-does-not-exist.json".to_owned(),
        "--json".to_owned(),
    ]);

    assert_eq!(output.status, 1);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.contains("viewer export"));
}

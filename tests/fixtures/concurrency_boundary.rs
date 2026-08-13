#[path = "concurrency_fixture_api.rs"]
mod fixture;

#[test]
fn minimum_case_has_a_deterministic_admission_trace() {
    let case = fixture::case("minimum").expect("minimum fixture case");
    let evaluation = fixture::parse_and_trace(&case).expect("minimum case should parse");

    assert_eq!(evaluation.parsed.max_repositories, 1);
    assert_eq!(evaluation.parsed.max_child_work, 1);
    assert_eq!(
        evaluation.trace,
        vec![
            "repository.queued:1".to_owned(),
            "repository.admitted:1".to_owned(),
            "child.queued:1".to_owned(),
            "child.admitted:1".to_owned(),
            "child.released:0".to_owned(),
            "repository.released:0".to_owned(),
        ]
    );
}

#[test]
fn every_boundary_case_matches_its_expected_parse_and_trace() {
    let cases = fixture::cases();
    assert!(
        cases.len() >= 20,
        "boundary table should remain broad, found {} rows",
        cases.len()
    );

    for case in cases {
        let actual = fixture::parse_and_trace(&case);
        match case.expected {
            fixture::Expected::Parsed {
                max_repositories,
                max_child_work,
                trace,
            } => {
                let evaluation = actual
                    .unwrap_or_else(|error| panic!("{} unexpectedly failed: {error:?}", case.name));
                assert_eq!(
                    evaluation.parsed.max_repositories, max_repositories,
                    "{} parsed repository cap",
                    case.name
                );
                assert_eq!(
                    evaluation.parsed.max_child_work, max_child_work,
                    "{} parsed child cap",
                    case.name
                );
                assert_eq!(evaluation.trace, trace, "{} admission trace", case.name);
            }
            fixture::Expected::Error { field, kind } => {
                let error = actual.expect_err(&format!("{} unexpectedly parsed", case.name));
                assert_eq!(error.field, field, "{} error field", case.name);
                assert_eq!(error.kind, kind, "{} error kind", case.name);
            }
        }
    }
}

#[test]
fn table_names_cover_each_required_boundary() {
    let names: Vec<String> = fixture::cases().into_iter().map(|case| case.name).collect();

    for required in [
        "minimum",
        "maximum",
        "omitted",
        "zero-repositories",
        "zero-child-work",
        "negative-repositories",
        "fractional-repositories",
        "noninteger-child-work",
        "null-repositories",
        "out-of-range-repositories",
        "unknown-field",
        "duplicate-field",
        "lower-repository-override",
        "lower-child-override",
        "nested-saturation",
        "cancellation-queued",
        "cancellation-active",
    ] {
        assert!(
            names.iter().any(|name| name == required),
            "missing {required}"
        );
    }
}

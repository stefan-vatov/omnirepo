use fixture::*;
use omnirepo_test_support::cross_domain_fixture as fixture;

#[test]
fn tracer_bullet_round_trip_is_versioned_and_replayable() {
    let spec = FixtureSpec::new("machine-source-roundtrip", 74_202).expect("valid case");
    let fixture = CrossDomainFixture::new(spec);
    let bytes = fixture.to_bytes();
    let context = FixtureContext::new("identity", "machine-source-roundtrip", 74_202);
    let replayed = CrossDomainFixture::from_bytes(&context, &bytes).expect("fixture should decode");

    assert_eq!(fixture, replayed);
    assert_eq!(bytes, replayed.to_bytes());
    assert_eq!(
        fixture.identity().contract_version(),
        CROSS_DOMAIN_FIXTURE_CONTRACT_VERSION
    );
    assert_eq!(fixture.identity().case_id(), "machine-source-roundtrip");
    assert_eq!(fixture.identity().seed(), 74_202);
    assert!(fixture.identity().fixture_id().starts_with("fixture-"));
}

#[test]
fn generated_fixture_exposes_each_cross_domain_contract_without_effects() {
    let fixture = CrossDomainFixture::new(FixtureSpec::new("all-domains", 74_206).unwrap());

    assert_eq!(fixture.machine().version(), 1);
    assert_eq!(fixture.catalog().entries().len(), 3);
    assert!(matches!(
        fixture.policy().presence(),
        PolicyPresenceFixture::Present(_)
    ));
    assert_eq!(fixture.plan().entries().len(), 1);
    assert!(matches!(
        fixture.content().mode(),
        ContentModeFixture::PartialSection { .. }
    ));
    assert_eq!(fixture.path().compare(), PathComparisonFixture::SameObject);
    assert_eq!(fixture.snapshot().state(), SnapshotStateFixture::Published);
    assert_eq!(
        fixture.delta().classification(),
        &DeltaClassificationFixture::Authorized
    );
    assert_eq!(fixture.journal().events().len(), 2);
    assert_eq!(fixture.process().status().label, "success");
    assert!(matches!(
        fixture.repair().eligibility(),
        RepairEligibilityFixture::Eligible { next_attempt: 1 }
    ));
    assert_eq!(fixture.cli().status().code(), 0);
    assert_eq!(fixture.release().tag(), "v0.8.3");
    assert_eq!(
        fixture.context(),
        FixtureContext {
            module: "cross-domain",
            case_id: "all-domains".to_owned(),
            seed: 74_206,
        }
    );
}

#[test]
fn contract_rows_are_stable_and_link_to_canonical_and_supporting_owners() {
    let context = FixtureContext::new("traceability", "canonical-contract-rows", 74_300);
    let rows = contract_cases(&context).expect("canonical matrix rows should decode");
    assert_eq!(rows.len(), 15);
    let mut case_ids = Vec::new();
    let mut row_ids = Vec::new();
    let mut fixture_ids = Vec::new();
    let mut evidence_ids = Vec::new();
    let mut replay_ids = Vec::new();
    for row in &rows {
        assert!(case_ids.iter().all(|id| id != row.case_id()));
        assert!(row_ids.iter().all(|id| id != row.row_id()));
        assert!(fixture_ids.iter().all(|id| id != row.fixture_id()));
        assert!(evidence_ids.iter().all(|id| id != row.evidence_id()));
        assert!(replay_ids.iter().all(|id| id != row.replay_id()));
        case_ids.push(row.case_id().to_owned());
        row_ids.push(row.row_id().to_owned());
        fixture_ids.push(row.fixture_id().to_owned());
        evidence_ids.push(row.evidence_id().to_owned());
        replay_ids.push(row.replay_id().to_owned());
        assert_eq!(row.primary_owner(), row.implementation_bead());
        let downstream = row.downstream_bead();
        assert!(
            downstream == "omni-constitutional-convergence-2r9"
                || downstream.starts_with("omni-constitutional-convergence-2r9."),
            "downstream bead must be the epic or one of its descendants: {downstream}"
        );
        assert_eq!(
            contract_case(&context, row.case_id()).unwrap().as_ref(),
            Some(row)
        );
    }
    assert_eq!(
        case_ids[0], "trace.behavior.configuration-authority",
        "case order is part of the deterministic fixture table"
    );
    let configuration = contract_case(&context, "trace.behavior.configuration-authority")
        .expect("canonical configuration row should decode")
        .expect("canonical configuration row should be selected");
    assert_eq!(
        configuration.fixture_id(),
        "fixture:machine-source-destination-config"
    );
    assert_eq!(
        configuration.evidence_id(),
        "evidence.trace.behavior.configuration-authority.v1"
    );
    assert_eq!(
        configuration.replay_id(),
        "replay.trace.behavior.configuration-authority.v1"
    );
    assert_eq!(
        configuration.primary_owner(),
        "omni-constitutional-convergence-2r9.3"
    );
    let precedence = contract_case(&context, "trace.principle.declared-precedence")
        .expect("canonical precedence row should decode")
        .expect("canonical precedence row should be selected");
    assert_eq!(precedence.reference(), "constitution:principle.6");
    assert_eq!(
        precedence.primary_owner(),
        "omni-constitutional-convergence-2r9.4"
    );
    assert_ne!(
        precedence.primary_owner(),
        "omni-constitutional-convergence-2r9.74.1",
        "stale local ownership must not replace canonical precedence ownership"
    );
}

#[test]
fn fixed_case_and_seed_produce_byte_identical_fixture() {
    let first =
        CrossDomainFixture::new(FixtureSpec::new("stable-identity", 74_203).expect("valid case"));
    let second =
        CrossDomainFixture::new(FixtureSpec::new("stable-identity", 74_203).expect("valid case"));
    assert_eq!(first.to_bytes(), second.to_bytes());
    assert_ne!(
        first.to_bytes(),
        CrossDomainFixture::new(FixtureSpec::new("stable-identity", 74_204).unwrap()).to_bytes()
    );
}

#[test]
fn malformed_fixture_errors_identify_case_and_seed_when_available() {
    let fixture = CrossDomainFixture::new(FixtureSpec::new("failure-context", 74_205).unwrap());
    let context = FixtureContext::new("identity", "failure-context", 74_205);
    let mut bytes = fixture.to_bytes();
    let marker = b"payload_digest=";
    let start = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .expect("digest field");
    bytes[start + marker.len()] = if bytes[start + marker.len()] == b'0' {
        b'1'
    } else {
        b'0'
    };
    let error =
        CrossDomainFixture::from_bytes(&context, &bytes).expect_err("digest mutation must fail");
    assert_eq!(error.module, "identity");
    assert_eq!(error.case_id, "failure-context");
    assert_eq!(error.seed, 74_205);
    assert!(error.reason.contains("payload digest mismatch"));
    assert!(error.to_string().contains("case_id=failure-context"));
    assert!(error.to_string().contains("seed=74205"));

    let diagnostic = fixture.failure("plan", "collision in fixture table");
    assert_eq!(diagnostic.module, "plan");
    assert_eq!(diagnostic.case_id, "failure-context");
    assert_eq!(diagnostic.seed, 74_205);
    assert!(
        diagnostic
            .to_string()
            .contains("collision in fixture table")
    );
}

#[test]
fn fixture_schema_and_bytes_are_strictly_versioned() {
    let fixture = CrossDomainFixture::new(FixtureSpec::new("strict-schema", 74_208).unwrap());
    let schema_context = FixtureContext::new("identity", "strict-schema", 74_208);
    let bytes = fixture.to_bytes();

    let mut unsupported = bytes.clone();
    let schema = b"schema=cross-domain-fixtures/v1";
    let replacement = b"schema=cross-domain-fixtures/v9";
    let offset = unsupported
        .windows(schema.len())
        .position(|window| window == schema)
        .expect("schema field");
    unsupported[offset..offset + schema.len()].copy_from_slice(replacement);
    let schema_error =
        CrossDomainFixture::from_bytes(&schema_context, &unsupported).expect_err("schema drift");
    assert!(schema_error.reason.contains("unsupported fixture schema"));

    let without_final_newline = &bytes[..bytes.len() - 1];
    let canonical_error = CrossDomainFixture::from_bytes(&schema_context, without_final_newline)
        .expect_err("non-canonical bytes");
    assert!(canonical_error.reason.contains("canonical byte order"));

    let machine_context = FixtureContext::new("machine-configuration", "strict-schema", 74_208);
    let schema_limits = MachineLimitsFixture::new(&machine_context, 4, 8).unwrap();
    let invalid_machine = MachineConfigurationFixture::new(
        &machine_context,
        2,
        vec!["source".to_owned()],
        vec!["repository".to_owned()],
        schema_limits,
        vec!["source".to_owned()],
        Vec::new(),
        0,
    )
    .expect_err("future machine schema");
    assert!(
        invalid_machine
            .reason
            .contains("unsupported schema version")
    );

    let invalid_case = FixtureSpec::new("UpperCase", 74_208).expect_err("case IDs are exact");
    assert_eq!(invalid_case.module, "identity");
    assert_eq!(invalid_case.seed, 74_208);
}

#[test]
fn source_order_is_the_only_precedence_tiebreaker_and_collision_is_explicit() {
    let context = FixtureContext::new("source-catalog", "precedence", 74_209);
    let whole = SourceModeFixture::WholeFile;
    let compatible = SourceCatalogFixture::new(
        &context,
        vec![
            SourceDeclarationFixture::new(
                &context,
                "source.high",
                "item",
                "config/file",
                whole.clone(),
            )
            .unwrap(),
            SourceDeclarationFixture::new(
                &context,
                "source.low",
                "item",
                "config/file",
                whole.clone(),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    assert_eq!(
        compatible.resolve("item"),
        CatalogResolutionFixture::Resolved {
            winner: "source.high".to_owned(),
            shadowed: vec!["source.low".to_owned()],
        }
    );
    let reversed = SourceCatalogFixture::with_source_order(
        &context,
        vec!["source.low".to_owned(), "source.high".to_owned()],
        compatible.entries().to_vec(),
    )
    .unwrap();
    assert_eq!(
        reversed.resolve("item"),
        CatalogResolutionFixture::Resolved {
            winner: "source.low".to_owned(),
            shadowed: vec!["source.high".to_owned()],
        }
    );

    let destination_collision = SourceCatalogFixture::new(
        &context,
        vec![
            SourceDeclarationFixture::new(
                &context,
                "source.high",
                "item",
                "config/high",
                whole.clone(),
            )
            .unwrap(),
            SourceDeclarationFixture::new(
                &context,
                "source.low",
                "item",
                "config/low",
                whole.clone(),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    assert!(matches!(
        destination_collision.resolve("item"),
        CatalogResolutionFixture::Collision { .. }
    ));

    let mode_collision = SourceCatalogFixture::new(
        &context,
        vec![
            SourceDeclarationFixture::new(&context, "source.high", "item", "config/file", whole)
                .unwrap(),
            SourceDeclarationFixture::new(
                &context,
                "source.low",
                "item",
                "config/file",
                SourceModeFixture::partial(&context, "settings").unwrap(),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    assert!(matches!(
        mode_collision.resolve("item"),
        CatalogResolutionFixture::Collision { .. }
    ));
    assert_eq!(
        mode_collision.resolve("missing"),
        CatalogResolutionFixture::NotFound
    );
}

#[test]
fn policy_presence_does_not_turn_empty_or_invalid_policy_into_inference() {
    let context = FixtureContext::new("repository-policy", "policy-presence", 74_210);
    let command = VerificationCommandFixture::new(&context, ["check"]).unwrap();
    let present = RepositoryPolicyFixture::present(
        PolicyDocumentFixture::new(
            &context,
            false,
            vec!["item.selected".to_owned()],
            vec!["item.excluded".to_owned()],
            vec![command],
        )
        .unwrap(),
    );
    assert_eq!(
        present.decision("item.selected"),
        PolicyDecisionFixture::Selected
    );
    assert_eq!(
        present.decision("item.excluded"),
        PolicyDecisionFixture::NotSelected
    );
    assert_eq!(
        present.decision("item.other"),
        PolicyDecisionFixture::NotSelected
    );

    let empty_present = RepositoryPolicyFixture::present(
        PolicyDocumentFixture::new(&context, false, Vec::new(), Vec::new(), Vec::new()).unwrap(),
    );
    assert_eq!(
        empty_present.decision("item.other"),
        PolicyDecisionFixture::NotSelected
    );
    assert_eq!(
        RepositoryPolicyFixture::absent().decision("item.other"),
        PolicyDecisionFixture::Infer
    );
    assert!(matches!(
        RepositoryPolicyFixture::invalid("duplicate key").decision("item.other"),
        PolicyDecisionFixture::Rejected { .. }
    ));
    assert!(matches!(
        RepositoryPolicyFixture::ambiguous("two authority files").decision("item.other"),
        PolicyDecisionFixture::Rejected { .. }
    ));
    let exclusion_wins = RepositoryPolicyFixture::present(
        PolicyDocumentFixture::new(
            &context,
            false,
            vec!["same".to_owned()],
            vec!["same".to_owned()],
            Vec::new(),
        )
        .unwrap(),
    );
    assert_eq!(
        exclusion_wins.decision("same"),
        PolicyDecisionFixture::NotSelected
    );
}

#[test]
fn marker_topology_and_byte_decisions_are_table_driven() {
    let context = FixtureContext::new("content-topology", "marker-table", 74_211);
    let source = b"# omnirepo:start section\nsource\n# omnirepo:end section\n".to_vec();
    let invalid_cases = [
        (
            b"# omnirepo:start section\n".as_slice(),
            MarkerTopologyFixture::MissingEnd,
        ),
        (
            b"# omnirepo:end section\n".as_slice(),
            MarkerTopologyFixture::MissingStart,
        ),
        (
            b"# omnirepo:end section\n# omnirepo:start section\n".as_slice(),
            MarkerTopologyFixture::Reversed,
        ),
    ];
    for (destination, topology) in invalid_cases {
        let content = ContentFixture::new(
            &context,
            ContentModeFixture::partial(&context, "section").unwrap(),
            source.clone(),
            destination.to_vec(),
        )
        .unwrap();
        assert_eq!(
            content.decision(),
            ContentDecisionFixture::InvalidMarkers(topology)
        );
        assert!(!topology.is_valid());
    }

    let whole_equal = ContentFixture::new(
        &context,
        ContentModeFixture::WholeFile,
        vec![0, 255, b'\n'],
        vec![0, 255, b'\n'],
    )
    .unwrap();
    assert_eq!(whole_equal.decision(), ContentDecisionFixture::Unchanged);
    assert!(!whole_equal.source_is_utf8());

    let whole_changed = ContentFixture::new(
        &context,
        ContentModeFixture::WholeFile,
        b"source\r\n".to_vec(),
        b"destination\n".to_vec(),
    )
    .unwrap();
    assert_eq!(
        whole_changed.decision(),
        ContentDecisionFixture::ReplaceWholeFile
    );

    let partial_append = ContentFixture::new(
        &context,
        ContentModeFixture::partial(&context, "section").unwrap(),
        source,
        Vec::new(),
    )
    .unwrap();
    assert_eq!(
        partial_append.decision(),
        ContentDecisionFixture::AppendPartialSection
    );
    assert_eq!(
        partial_append.marker_lines(),
        Some((
            "# omnirepo:start section".to_owned(),
            "# omnirepo:end section".to_owned(),
        ))
    );
}

#[test]
fn partial_destinations_without_markers_append_and_preserve_all_local_bytes() {
    let sources = [
        b"# omnirepo:start section\nsource=true\n# omnirepo:end section\n".as_slice(),
        b"# omnirepo:start section\r\nsource=true\r\n# omnirepo:end section\r\n".as_slice(),
        b"# omnirepo:start section\nsource=true\n# omnirepo:end section".as_slice(),
    ];
    let destinations = [
        b"".as_slice(),
        b"local=true\n".as_slice(),
        b"local=true\r\n".as_slice(),
        b"local=true".as_slice(),
        &[0, 255, b'\n'][..],
    ];

    for (source_index, source) in sources.into_iter().enumerate() {
        for (destination_index, destination) in destinations.into_iter().enumerate() {
            let case_id = format!("partial-append-{source_index}-{destination_index}");
            let case_context = FixtureContext::new("content-topology", case_id, 74_416);
            let content = ContentFixture::new(
                &case_context,
                ContentModeFixture::partial(&case_context, "section").unwrap(),
                source.to_vec(),
                destination.to_vec(),
            )
            .expect("valid paired source and marker-free destination");
            assert_eq!(
                content.marker_topology(),
                MarkerTopologyFixture::Absent,
                "case_id={} seed={}",
                case_context.case_id,
                case_context.seed
            );
            assert_eq!(
                content.decision(),
                ContentDecisionFixture::AppendPartialSection,
                "case_id={} seed={}",
                case_context.case_id,
                case_context.seed
            );
            assert_eq!(content.destination_bytes(), destination);
        }
    }
}

#[test]
fn partial_sources_require_one_valid_pair_and_fail_closed_for_absent_or_invalid_topology() {
    let destination = b"local=true\n";
    let cases = [
        ("absent", b"source=true\n".as_slice(), MarkerTopologyFixture::Absent),
        (
            "missing-end",
            b"# omnirepo:start section\nsource=true\n".as_slice(),
            MarkerTopologyFixture::MissingEnd,
        ),
        (
            "reversed",
            b"# omnirepo:end section\nsource=true\n# omnirepo:start section\n".as_slice(),
            MarkerTopologyFixture::Reversed,
        ),
        (
            "duplicate",
            b"# omnirepo:start section\n# omnirepo:start section\nsource=true\n# omnirepo:end section\n".as_slice(),
            MarkerTopologyFixture::Duplicate,
        ),
        (
            "nested",
            b"# omnirepo:start outer\n# omnirepo:start section\nsource=true\n# omnirepo:end section\n# omnirepo:end outer\n".as_slice(),
            MarkerTopologyFixture::Nested,
        ),
        (
            "unknown-marker-like",
            b"# omnirepo unknown\n".as_slice(),
            MarkerTopologyFixture::Unknown,
        ),
    ];

    for (case_id, source, expected) in cases {
        let context = FixtureContext::new("content-topology", case_id, 74_417);
        let content = ContentFixture::new(
            &context,
            ContentModeFixture::partial(&context, "section").unwrap(),
            source.to_vec(),
            destination.to_vec(),
        )
        .expect("invalid source marker bytes remain representable as fixture data");
        assert_eq!(content.marker_topology(), expected, "case_id={case_id}");
        assert_eq!(
            content.decision(),
            ContentDecisionFixture::InvalidMarkers(expected),
            "case_id={case_id} seed={}",
            context.seed
        );
        assert_eq!(content.destination_bytes(), destination);
    }
}

#[test]
fn path_identity_uses_filesystem_and_object_identity_not_lexical_text() {
    let context = FixtureContext::new("path-identity", "identity-comparison", 74_212);
    let filesystem =
        FilesystemIdentityFixture::new(&context, FilesystemKindFixture::LinuxExt, 7, 11).unwrap();
    let root = AuthorityIdentityFixture::new(
        &context,
        "/fixture/root",
        filesystem,
        ObjectIdentityFixture::new(7, 100),
    )
    .unwrap();
    let alias = AuthorityIdentityFixture::new(
        &context,
        "/fixture/alias",
        filesystem,
        ObjectIdentityFixture::new(7, 100),
    )
    .unwrap();
    assert_eq!(
        PathIdentityFixture::new(&context, root.clone(), alias)
            .unwrap()
            .compare(),
        PathComparisonFixture::SameObject
    );
    let other = AuthorityIdentityFixture::new(
        &context,
        "/fixture/other",
        filesystem,
        ObjectIdentityFixture::new(7, 101),
    )
    .unwrap();
    assert_eq!(
        PathIdentityFixture::new(&context, root, other)
            .unwrap()
            .compare(),
        PathComparisonFixture::DistinctObject
    );
    let error = AuthorityIdentityFixture::new(
        &context,
        "/fixture/bad",
        filesystem,
        ObjectIdentityFixture::new(9, 100),
    )
    .expect_err("device mismatch must fail closed");
    assert!(error.reason.contains("device mismatch"));
}

#[test]
fn snapshot_transitions_accept_only_declared_edges() {
    let valid = [
        (
            SnapshotStateFixture::Empty,
            SnapshotEventFixture::BeginAcquire,
            SnapshotStateFixture::Acquiring,
        ),
        (
            SnapshotStateFixture::Acquiring,
            SnapshotEventFixture::StageReady,
            SnapshotStateFixture::Staged,
        ),
        (
            SnapshotStateFixture::Staged,
            SnapshotEventFixture::Publish,
            SnapshotStateFixture::Published,
        ),
        (
            SnapshotStateFixture::Staged,
            SnapshotEventFixture::Fail,
            SnapshotStateFixture::Failed,
        ),
        (
            SnapshotStateFixture::Staged,
            SnapshotEventFixture::Interrupt,
            SnapshotStateFixture::Interrupted,
        ),
        (
            SnapshotStateFixture::Failed,
            SnapshotEventFixture::Reset,
            SnapshotStateFixture::Empty,
        ),
    ];
    for (state, event, expected) in valid {
        assert_eq!(transition_snapshot(state, event), Ok(expected));
    }
    for (state, event) in [
        (SnapshotStateFixture::Empty, SnapshotEventFixture::Publish),
        (SnapshotStateFixture::Published, SnapshotEventFixture::Reset),
        (SnapshotStateFixture::Failed, SnapshotEventFixture::Publish),
    ] {
        let error = transition_snapshot(state, event).expect_err("undeclared edge");
        assert_eq!(error.from, state);
        assert_eq!(error.event, event);
    }
}

#[test]
fn authorized_delta_classification_rejects_scope_escape_and_collisions() {
    let context = FixtureContext::new("authorized-delta", "delta-classification", 74_213);
    let target =
        FileChangeFixture::new(&context, "config/file", b"old".to_vec(), b"new".to_vec()).unwrap();
    let authorized = AuthorizedDeltaFixture::classify(
        &context,
        vec!["config/file".to_owned()],
        vec![target.clone()],
    )
    .unwrap();
    assert_eq!(
        authorized.classification(),
        &DeltaClassificationFixture::Authorized
    );

    let unauthorized = AuthorizedDeltaFixture::classify(
        &context,
        vec!["config/file".to_owned()],
        vec![
            FileChangeFixture::new(&context, "README.md", Vec::new(), b"changed".to_vec()).unwrap(),
        ],
    )
    .unwrap();
    assert_eq!(
        unauthorized.classification(),
        &DeltaClassificationFixture::Unauthorized {
            path: "README.md".to_owned()
        }
    );

    let collision = AuthorizedDeltaFixture::classify(
        &context,
        vec!["config/file".to_owned()],
        vec![target.clone(), target],
    )
    .unwrap();
    assert_eq!(
        collision.classification(),
        &DeltaClassificationFixture::Collision {
            path: "config/file".to_owned()
        }
    );
    assert!(
        AuthorizedDeltaFixture::classify(
            &context,
            vec!["config/file".to_owned(), "config/file".to_owned()],
            Vec::new()
        )
        .is_err()
    );
}

#[test]
fn journal_events_retain_case_seed_and_monotonic_state() {
    let context = FixtureContext::new("journal", "journal-case", 74_207);
    let events = vec![
        JournalEventFixture::new(
            &context,
            1,
            "journal-case",
            74_207,
            "sync.start",
            JournalOutcomeFixture::Started,
        )
        .unwrap(),
        JournalEventFixture::new(
            &context,
            2,
            "journal-case",
            74_207,
            "sync.cancel",
            JournalOutcomeFixture::Cancelled,
        )
        .unwrap(),
    ];
    let journal = JournalFixture::new(&context, events).unwrap();
    assert_eq!(journal.events()[1].case_id(), "journal-case");
    assert_eq!(journal.events()[1].seed(), 74_207);
    assert_eq!(
        journal.events()[1].outcome(),
        JournalOutcomeFixture::Cancelled
    );
    assert!(
        JournalFixture::new(
            &context,
            vec![
                JournalEventFixture::new(
                    &context,
                    2,
                    "bad-sequence",
                    1,
                    "stage",
                    JournalOutcomeFixture::Started
                )
                .unwrap(),
            ]
        )
        .is_err()
    );
}

#[test]
fn process_and_cli_status_mapping_preserves_failure_categories() {
    let cases = [
        (ProcessDispositionFixture::Success, "success"),
        (
            ProcessDispositionFixture::ExitFailure { code: 7 },
            "exit-failure",
        ),
        (
            ProcessDispositionFixture::Signaled { signal: 15 },
            "signaled",
        ),
        (ProcessDispositionFixture::TimedOut, "timeout"),
        (ProcessDispositionFixture::Cancelled, "cancelled"),
        (
            ProcessDispositionFixture::SpawnFailed {
                reason: "missing".to_owned(),
            },
            "spawn-failed",
        ),
    ];
    for (outcome, label) in cases {
        assert_eq!(outcome.label(), label);
    }
    assert_eq!(
        cli_status_for_process(&ProcessDispositionFixture::Success, 2, 2, true),
        CliCodeFixture::Success
    );
    assert_eq!(
        cli_status_for_process(
            &ProcessDispositionFixture::ExitFailure { code: 1 },
            2,
            1,
            true
        ),
        CliCodeFixture::PartialFailure
    );
    assert_eq!(
        cli_status_for_process(&ProcessDispositionFixture::TimedOut, 2, 0, true),
        CliCodeFixture::AllFailed
    );
    assert_eq!(
        cli_status_for_process(&ProcessDispositionFixture::Cancelled, 2, 1, true),
        CliCodeFixture::Cancelled
    );
    assert_eq!(
        cli_status_for_process(&ProcessDispositionFixture::Success, 0, 0, true),
        CliCodeFixture::InvocationError
    );
    assert_eq!(
        cli_status_for_process(&ProcessDispositionFixture::Success, 2, 2, false),
        CliCodeFixture::RecordFailure
    );
    let codes = [
        (CliCodeFixture::Success, 0),
        (CliCodeFixture::InvocationError, 2),
        (CliCodeFixture::PartialFailure, 3),
        (CliCodeFixture::AllFailed, 4),
        (CliCodeFixture::RecordFailure, 5),
        (CliCodeFixture::Cancelled, 130),
    ];
    for (status, code) in codes {
        assert_eq!(status.code(), code);
        assert!(!status.label().is_empty());
    }
}

#[test]
fn repair_eligibility_requires_causation_and_remaining_attempts() {
    let context = FixtureContext::new("repair", "repair-table", 74_214);
    let cases = [
        (
            RepairInputFixture::new(&context, RepairCausationFixture::CurrentManagedPath, 0, 3)
                .unwrap(),
            RepairEligibilityFixture::Eligible { next_attempt: 1 },
        ),
        (
            RepairInputFixture::new(&context, RepairCausationFixture::PriorPassingBaseline, 2, 3)
                .unwrap(),
            RepairEligibilityFixture::Eligible { next_attempt: 3 },
        ),
        (
            RepairInputFixture::new(&context, RepairCausationFixture::CurrentManagedPath, 3, 3)
                .unwrap(),
            RepairEligibilityFixture::Ineligible {
                reason: "repair attempts are exhausted",
            },
        ),
        (
            RepairInputFixture::new(&context, RepairCausationFixture::Unrelated, 0, 3).unwrap(),
            RepairEligibilityFixture::Ineligible {
                reason: "causation is not established",
            },
        ),
        (
            RepairInputFixture::new(&context, RepairCausationFixture::Unknown, 0, 3).unwrap(),
            RepairEligibilityFixture::Ineligible {
                reason: "causation is not established",
            },
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(repair_eligibility(input), expected);
    }
}

#[test]
fn release_identity_binds_semver_tag_commit_and_digest() {
    let context = FixtureContext::new("release", "release-table", 74_215);
    let commit = "0123456789abcdef0123456789abcdef01234567";
    let digest = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let release = ReleaseIdentityFixture::new(&context, "1.2.3", "v1.2.3", commit, digest)
        .expect("valid release identity");
    assert_eq!(release.version(), "1.2.3");
    assert_eq!(release.tag(), "v1.2.3");
    assert!(ReleaseIdentityFixture::new(&context, "1.2", "v1.2", commit, digest).is_err());
    assert!(ReleaseIdentityFixture::new(&context, "1.2.3", "v9.9.9", commit, digest).is_err());
    assert!(ReleaseIdentityFixture::new(&context, "1.2.3", "v1.2.3", "", digest).is_err());
}

#[test]
fn deterministic_seed_property_replays_a_range_without_ambient_state() {
    for seed in 0..32_u64 {
        let case_id = format!("property-case-{seed}");
        let first = CrossDomainFixture::new(FixtureSpec::new(case_id.clone(), seed).unwrap());
        let second = CrossDomainFixture::new(FixtureSpec::new(case_id.clone(), seed).unwrap());
        assert_eq!(
            first.to_bytes(),
            second.to_bytes(),
            "case_id={case_id} seed={seed}"
        );
        assert_eq!(
            CrossDomainFixture::from_bytes(
                &FixtureContext::new("identity", case_id.clone(), seed),
                &first.to_bytes(),
            )
            .unwrap(),
            first,
            "case_id={case_id} seed={seed} round trip"
        );
    }
}

#[test]
fn constructor_failures_preserve_caller_identity_and_reject_unsafe_authority_aliases() {
    let context = FixtureContext::new("path-identity", "hostile-paths", 74_400);
    let unsupported =
        FilesystemIdentityFixture::new(&context, FilesystemKindFixture::Unsupported, 7, 11)
            .expect_err("unsupported filesystems must fail closed");
    assert_eq!(unsupported.case_id, "hostile-paths");
    assert_eq!(unsupported.seed, 74_400);
    assert!(unsupported.reason.contains("unsupported filesystem"));

    let filesystem =
        FilesystemIdentityFixture::new(&context, FilesystemKindFixture::LinuxExt, 7, 11)
            .expect("supported filesystem");
    let parent = AuthorityIdentityFixture::new(
        &context,
        "/fixture/../escape",
        filesystem,
        ObjectIdentityFixture::new(7, 100),
    )
    .expect_err("parent traversal must fail closed");
    assert_eq!(parent.case_id, "hostile-paths");
    assert_eq!(parent.seed, 74_400);
    assert!(parent.reason.contains("parent"));

    let absolute_destination = SourceDeclarationFixture::new(
        &context,
        "source",
        "item",
        "/fixture/absolute-nested",
        SourceModeFixture::WholeFile,
    )
    .expect_err("absolute nested destinations must fail closed");
    assert_eq!(absolute_destination.case_id, "hostile-paths");
    assert_eq!(absolute_destination.seed, 74_400);
    assert!(
        absolute_destination
            .reason
            .contains("contained destination")
    );

    let authority = AuthorityIdentityFixture::new(
        &context,
        "/fixture/root",
        filesystem,
        ObjectIdentityFixture::new(7, 100),
    )
    .expect("regular authority");
    for (kind, reason) in [
        (ObjectKindFixture::Symlink, "symlink"),
        (ObjectKindFixture::Mount, "mount"),
        (ObjectKindFixture::HardLink, "hard link"),
        (ObjectKindFixture::NonRegular, "non-regular"),
    ] {
        let alias = AuthorityIdentityFixture::new(
            &context,
            "/fixture/alias",
            filesystem,
            ObjectIdentityFixture::with_kind(7, 100, kind),
        )
        .expect("alias identity is representable before comparison");
        let error = PathIdentityFixture::new(&context, authority.clone(), alias)
            .expect_err("unsafe alias must fail closed");
        assert_eq!(error.case_id, "hostile-paths");
        assert_eq!(error.seed, 74_400);
        assert!(error.reason.contains(reason));
    }
}

#[test]
fn repair_inputs_enforce_the_selected_maximum_and_counter_consistency() {
    let context = FixtureContext::new("repair", "repair-bounds", 74_401);
    let too_high =
        RepairInputFixture::new(&context, RepairCausationFixture::CurrentManagedPath, 0, 4)
            .expect_err("repair maximum above three must fail closed");
    assert_eq!(too_high.case_id, "repair-bounds");
    assert_eq!(too_high.seed, 74_401);
    assert!(too_high.reason.contains("maximum 3"));

    let inconsistent =
        RepairInputFixture::new(&context, RepairCausationFixture::CurrentManagedPath, 3, 2)
            .expect_err("used attempts cannot exceed the configured maximum");
    assert_eq!(inconsistent.case_id, "repair-bounds");
    assert_eq!(inconsistent.seed, 74_401);
    assert!(inconsistent.reason.contains("used"));

    let valid =
        RepairInputFixture::new(&context, RepairCausationFixture::PriorPassingBaseline, 2, 3)
            .expect("consistent bounded counters");
    assert_eq!(
        repair_eligibility(valid),
        RepairEligibilityFixture::Eligible { next_attempt: 3 }
    );
}

#[test]
fn release_identity_requires_strict_semver_and_full_canonical_digests() {
    let context = FixtureContext::new("release", "release-identity", 74_402);
    let commit = "0123456789abcdef0123456789abcdef01234567";
    let digest = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let release = ReleaseIdentityFixture::new(&context, "1.2.3", "v1.2.3", commit, digest)
        .expect("canonical release identity");
    assert_eq!(release.commit(), commit);
    assert_eq!(release.digest(), digest);

    for version in ["01.2.3", "1.02.3", "1.2.03", "1.2", "1.2.3\n"] {
        assert!(
            ReleaseIdentityFixture::new(&context, version, "v1.2.3", commit, digest).is_err(),
            "version {version:?} must fail closed"
        );
    }
    for (bad_commit, bad_digest) in [
        ("commit", digest),
        ("0123456789abcdef0123456789abcdef0123456", digest),
        (commit, "sha256:digest"),
        (
            commit,
            "sha512:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        ),
        (
            commit,
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n",
        ),
    ] {
        let error =
            ReleaseIdentityFixture::new(&context, "1.2.3", "v1.2.3", bad_commit, bad_digest)
                .expect_err("non-canonical release identity must fail closed");
        assert_eq!(error.case_id, "release-identity");
        assert_eq!(error.seed, 74_402);
    }
}

#[test]
fn marker_topology_is_derived_from_exact_bytes_and_rejects_malformed_lines() {
    let context = FixtureContext::new("content-topology", "marker-bytes", 74_403);
    let mode = ContentModeFixture::partial(&context, "section").expect("valid section");
    let source = b"# omnirepo:start section\nsource=true\n# omnirepo:end section\n".to_vec();
    let cases = [
        (
            "missing-start",
            b"payload\n# omnirepo:end section\n".as_slice(),
            MarkerTopologyFixture::MissingStart,
        ),
        (
            "missing-end",
            b"# omnirepo:start section\npayload\n".as_slice(),
            MarkerTopologyFixture::MissingEnd,
        ),
        (
            "reversed",
            b"# omnirepo:end section\npayload\n# omnirepo:start section\n".as_slice(),
            MarkerTopologyFixture::Reversed,
        ),
        (
            "duplicate",
            b"# omnirepo:start section\n# omnirepo:start section\npayload\n# omnirepo:end section\n".as_slice(),
            MarkerTopologyFixture::Duplicate,
        ),
        (
            "nested",
            b"# omnirepo:start outer\n# omnirepo:start section\npayload\n# omnirepo:end section\n# omnirepo:end outer\n".as_slice(),
            MarkerTopologyFixture::Nested,
        ),
        (
            "interleaved",
            b"# omnirepo:start outer\n# omnirepo:start section\npayload\n# omnirepo:end outer\n# omnirepo:end section\n".as_slice(),
            MarkerTopologyFixture::Interleaved,
        ),
        (
            "mismatched",
            b"# omnirepo:start section\npayload\n# omnirepo:end other\n".as_slice(),
            MarkerTopologyFixture::Mismatched,
        ),
        (
            "unknown",
            b"# omnirepo:middle section\n".as_slice(),
            MarkerTopologyFixture::Unknown,
        ),
        (
            "unknown-marker-like",
            b"# omnirepo unknown\n".as_slice(),
            MarkerTopologyFixture::Unknown,
        ),
        (
            "whitespace",
            b"#  omnirepo:start section\npayload\n# omnirepo:end section\n".as_slice(),
            MarkerTopologyFixture::WhitespaceAltered,
        ),
        (
            "payload-like",
            b"value=# omnirepo:start section\n".as_slice(),
            MarkerTopologyFixture::PayloadLike,
        ),
    ];
    for (case_id, destination, expected) in cases {
        let case_context = FixtureContext::new("content-topology", case_id, 74_403);
        let content = ContentFixture::new(
            &case_context,
            mode.clone(),
            source.clone(),
            destination.to_vec(),
        )
        .expect("invalid marker bytes remain representable as fixture data");
        assert_eq!(content.marker_topology(), expected, "case_id={case_id}");
        assert_eq!(
            content.decision(),
            ContentDecisionFixture::InvalidMarkers(expected)
        );
    }
    let unsupported_context = FixtureContext::new("content-topology", "marker-unsupported", 74_403);
    let unsupported = ContentFixture::new(
        &unsupported_context,
        mode,
        source,
        b"# omnirepo:start \xff\n".to_vec(),
    )
    .expect_err("non-UTF-8 marker bytes must fail closed");
    assert_eq!(unsupported.case_id, "marker-unsupported");
    assert_eq!(unsupported.seed, 74_403);
    assert!(unsupported.reason.contains("unsupported encoding"));
}

#[test]
fn machine_fixture_models_selected_limits_declarations_and_attempt_states() {
    let context = FixtureContext::new("machine-configuration", "machine-limits", 74_404);
    let limits = MachineLimitsFixture::new(&context, 4, 8).expect("selected defaults");
    let adapter = AdapterAttemptConfigFixture::new(
        &context,
        "codex",
        AdapterAttemptStateFixture::Configured,
        3,
    )
    .expect("configured adapter attempt");
    let machine = MachineConfigurationFixture::new(
        &context,
        1,
        vec!["source.high".to_owned()],
        vec!["repository".to_owned()],
        limits,
        vec!["source.high".to_owned()],
        vec![adapter],
        3,
    )
    .expect("complete machine fixture");
    assert_eq!(machine.limits().max_repositories(), 4);
    assert_eq!(machine.limits().max_child_work(), 8);
    assert_eq!(machine.source_declarations(), &["source.high".to_owned()]);
    assert_eq!(machine.adapter_attempts().len(), 1);

    let out_of_order = MachineConfigurationFixture::new(
        &context,
        1,
        vec!["source.high".to_owned(), "source.low".to_owned()],
        vec!["repository".to_owned()],
        limits,
        vec!["source.low".to_owned(), "source.high".to_owned()],
        Vec::new(),
        0,
    )
    .expect_err("source declaration order must remain authoritative");
    assert!(out_of_order.reason.contains("in order"));

    for invalid in [
        MachineLimitsFixture::new(&context, 0, 8),
        MachineLimitsFixture::new(&context, 33, 8),
        MachineLimitsFixture::new(&context, 4, 65),
    ] {
        assert!(invalid.is_err(), "machine limit must fail closed");
    }
    assert!(
        AdapterAttemptConfigFixture::new(
            &context,
            "codex",
            AdapterAttemptStateFixture::Disabled,
            1,
        )
        .is_err()
    );
}

#[test]
fn journal_fixture_rejects_invalid_transitions_and_requires_finality() {
    let context = FixtureContext::new("journal", "journal-finality", 74_405);
    let start = JournalEventFixture::new(
        &context,
        1,
        "journal-finality",
        74_405,
        "sync.start",
        JournalOutcomeFixture::Started,
    )
    .unwrap();
    let complete = JournalEventFixture::new(
        &context,
        2,
        "journal-finality",
        74_405,
        "sync.complete",
        JournalOutcomeFixture::Completed,
    )
    .unwrap();
    let journal = JournalFixture::new(&context, vec![start.clone(), complete.clone()])
        .expect("started -> completed is valid");
    assert!(journal.is_final());

    let late = JournalEventFixture::new(
        &context,
        3,
        "journal-finality",
        74_405,
        "sync.late",
        JournalOutcomeFixture::Started,
    )
    .unwrap();
    let error = JournalFixture::new(&context, vec![start, complete, late])
        .expect_err("terminal journal cannot accept later events");
    assert_eq!(error.case_id, "journal-finality");
    assert_eq!(error.seed, 74_405);
    assert!(error.reason.contains("terminal"));
}

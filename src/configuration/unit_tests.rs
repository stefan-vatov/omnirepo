use super::*;
/*
    AbsolutePath, AgentKind, ConfigurationError, DestinationRepository, MachineConcurrency,
    MachineConfiguration, RepairControls, RepositoryId, RepositoryTag, SchemaVersion, SourceId,
    SourceLocation, SourceReference,
*/

fn path(value: &str) -> AbsolutePath {
    AbsolutePath::parse(value).expect("fixture path must be valid")
}

fn repo(value: &str) -> DestinationRepository {
    DestinationRepository::new(
        RepositoryId::parse(value).unwrap(),
        path(&format!("/fleet/{value}")),
        [],
    )
    .unwrap()
}

fn source(value: &str, location: SourceLocation) -> SourceReference {
    SourceReference::new(SourceId::parse(value).unwrap(), location)
}

#[test]
fn version_one_and_defaults_are_explicit() {
    let config = MachineConfiguration::new(
        SchemaVersion::current(),
        vec![repo("app")],
        vec![source(
            "shared",
            SourceLocation::local(path("/sources/shared")),
        )],
        None,
        MachineConcurrency::default(),
        RepairControls::default(),
    )
    .unwrap();

    assert_eq!(config.version(), SchemaVersion::current());
    assert_eq!(config.repositories()[0].id().as_str(), "app");
    assert_eq!(config.sources()[0].id().as_str(), "shared");
    assert!(config.sources()[0].location().is_local());
    assert_eq!(config.sources()[0].location().as_str(), "/sources/shared");
    assert!(config.repositories()[0].tags().is_empty());
    assert_eq!(config.cache_root(), None);
    assert_eq!(config.concurrency().max_repositories(), 4);
    assert_eq!(config.concurrency().max_child_work(), 8);
    assert_eq!(config.repair().max_attempts(), 3);
    assert!(config.repair().priority().is_empty());
}

#[test]
fn source_order_is_retained_as_declared() {
    let first = source(
        "first",
        SourceLocation::remote("https://example.test/first").unwrap(),
    );
    let second = source(
        "second",
        SourceLocation::remote("ssh://git@example.test/second").unwrap(),
    );
    let config = MachineConfiguration::new(
        SchemaVersion::current(),
        vec![repo("app")],
        vec![first, second],
        Some(path("/cache")),
        MachineConcurrency::default(),
        RepairControls::default(),
    )
    .unwrap();

    assert_eq!(
        config
            .sources()
            .iter()
            .map(SourceReference::id)
            .map(SourceId::as_str)
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
}

#[test]
fn path_identity_is_lexical_until_authority_adapters_resolve_filesystem_identity() {
    let literal = path("/fleet/app");
    let lexical_alias = path("/fleet/./app");
    assert_ne!(literal, lexical_alias);

    let config = MachineConfiguration::new(
        SchemaVersion::current(),
        vec![
            DestinationRepository::new(RepositoryId::parse("one").unwrap(), literal, []).unwrap(),
            DestinationRepository::new(RepositoryId::parse("two").unwrap(), lexical_alias, [])
                .unwrap(),
        ],
        vec![],
        None,
        MachineConcurrency::default(),
        RepairControls::default(),
    )
    .unwrap();
    assert_eq!(config.repositories().len(), 2);
}

#[test]
fn schema_version_table_covers_current_and_unsupported_values() {
    let current = SchemaVersion::current();
    assert_eq!(current.value(), SUPPORTED_SCHEMA_VERSION);
    assert_eq!(SchemaVersion::new(SUPPORTED_SCHEMA_VERSION), Ok(current));

    for actual in [0, 2, u16::MAX] {
        let error = SchemaVersion::new(actual).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "unsupported configuration schema version {actual}; expected {SUPPORTED_SCHEMA_VERSION}"
            )
        );
    }
}

#[test]
fn slug_and_identity_tables_cover_ascii_boundaries_and_control_text() {
    let valid_slugs = ["a", "z9", "a.b", "a_b", "a-b", "0", "-leading", "trailing-"];
    for value in valid_slugs {
        assert!(
            SourceId::parse(value).is_ok(),
            "{value:?} should be a source id"
        );
        assert!(
            RepositoryTag::parse(value).is_ok(),
            "{value:?} should be a repository tag"
        );
    }

    let invalid_slugs = ["", "Upper", "space value", "slash/name", "a+b", "é", "a\t"];
    for value in invalid_slugs {
        let source_error = SourceId::parse(value).unwrap_err();
        assert_eq!(
            source_error.to_string(),
            format!(
                "invalid source id {value:?}; use lowercase ASCII letters, digits, '.', '_', or '-'"
            )
        );
        let tag_error = RepositoryTag::parse(value).unwrap_err();
        assert_eq!(
            tag_error.to_string(),
            format!(
                "invalid repository tag {value:?}; use lowercase ASCII letters, digits, '.', '_', or '-'"
            )
        );
    }

    for value in ["Project Alpha", "UPPER/with spaces", "repo.v1"] {
        assert!(
            RepositoryId::parse(value).is_ok(),
            "{value:?} should be an identity"
        );
    }
    for value in ["", "repo\nname", "repo\tname", "repo\0name"] {
        let error = RepositoryId::parse(value).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("invalid repository id {value:?}; control characters are not allowed")
        );
    }
}

#[test]
fn absolute_path_table_covers_valid_and_rejected_lexical_forms() {
    let valid = [
        "/",
        "/fleet/app",
        "/fleet/./app",
        "/fleet//app",
        "/fleet/...",
    ];
    for value in valid {
        assert!(
            AbsolutePath::parse(value).is_ok(),
            "{value:?} should be absolute"
        );
    }

    let invalid = [
        ("", "invalid path: path is empty"),
        ("relative/path", "invalid path: path must be absolute"),
        ("/fleet/\0bad", "invalid path: path contains NUL"),
        (
            "/fleet/../other",
            "invalid path: parent traversal is not allowed",
        ),
        ("/..", "invalid path: parent traversal is not allowed"),
    ];
    for (value, expected) in invalid {
        let error = AbsolutePath::parse(value).unwrap_err();
        assert_eq!(error.to_string(), expected);
        if !value.is_empty() {
            assert!(!error.to_string().contains(value));
        }
    }
}

#[test]
fn source_location_table_covers_local_and_supported_remote_transports() {
    let local = SourceLocation::local(path("/sources/local"));
    assert!(local.is_local());

    let supported = [
        "https://example.test/source",
        "ssh://git@example.test/source",
        "git@example.test:source",
    ];
    for value in supported {
        let remote = SourceLocation::remote(value).unwrap();
        assert!(!remote.is_local());
    }

    let unsupported = [
        "",
        "http://example.test/source",
        "file:///tmp/source",
        "git@example.test/source",
        "https://example.test/\0source",
    ];
    for value in unsupported {
        let error = SourceLocation::remote(value).unwrap_err();
        assert_eq!(
            error.to_string(),
            "invalid source location: source must use HTTPS or SSH"
        );
        if !value.is_empty() {
            assert!(!error.to_string().contains(value));
        }
    }
}

#[test]
fn duplicate_destination_tags_are_rejected_with_context() {
    let error = DestinationRepository::new(
        RepositoryId::parse("app").unwrap(),
        path("/fleet/app"),
        [
            RepositoryTag::parse("linux").unwrap(),
            RepositoryTag::parse("linux").unwrap(),
        ],
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "duplicate tag \"linux\" on repository \"app\""
    );
}

#[test]
fn repair_controls_table_covers_defaults_limits_and_each_agent_kind() {
    let defaults = RepairControls::default();
    assert_eq!(defaults.max_attempts(), DEFAULT_REPAIR_ATTEMPTS);

    for attempts in [0, 1, MAX_REPAIR_ATTEMPTS] {
        let controls = RepairControls::new(
            vec![AgentKind::Pi, AgentKind::Codex, AgentKind::ClaudeCode],
            attempts,
        )
        .unwrap();
        assert_eq!(
            controls.priority(),
            &[AgentKind::Pi, AgentKind::Codex, AgentKind::ClaudeCode]
        );
        assert_eq!(controls.max_attempts(), attempts);
    }

    let duplicate_cases = [
        (AgentKind::Codex, "duplicate repair agent Codex"),
        (AgentKind::ClaudeCode, "duplicate repair agent ClaudeCode"),
        (AgentKind::Pi, "duplicate repair agent Pi"),
    ];
    for (agent, expected) in duplicate_cases {
        let error = RepairControls::new(vec![agent, agent], 1).unwrap_err();
        assert_eq!(error.to_string(), expected);
    }

    let error = RepairControls::new(Vec::new(), MAX_REPAIR_ATTEMPTS + 1).unwrap_err();
    assert_eq!(
        error.to_string(),
        "repair-attempt ceiling 4 exceeds maximum 3"
    );
}

#[test]
fn concurrency_table_covers_defaults_minima_maxima_and_field_errors() {
    let defaults = MachineConcurrency::default();
    assert_eq!(defaults.max_repositories(), DEFAULT_MAX_REPOSITORIES);
    assert_eq!(defaults.max_child_work(), DEFAULT_MAX_CHILD_WORK);

    for (repositories, child_work) in [
        (1, 1),
        (1, MAX_CHILD_WORK),
        (MAX_REPOSITORIES, 1),
        (MAX_REPOSITORIES, MAX_CHILD_WORK),
    ] {
        let concurrency = MachineConcurrency::new(repositories, child_work).unwrap();
        assert_eq!(concurrency.max_repositories(), repositories);
        assert_eq!(concurrency.max_child_work(), child_work);
    }

    let invalid = [
        (
            "max_repositories",
            0,
            DEFAULT_MAX_CHILD_WORK,
            MAX_REPOSITORIES,
        ),
        (
            "max_repositories",
            MAX_REPOSITORIES + 1,
            DEFAULT_MAX_CHILD_WORK,
            MAX_REPOSITORIES,
        ),
        (
            "max_child_work",
            DEFAULT_MAX_REPOSITORIES,
            0,
            MAX_CHILD_WORK,
        ),
        (
            "max_child_work",
            DEFAULT_MAX_REPOSITORIES,
            MAX_CHILD_WORK + 1,
            MAX_CHILD_WORK,
        ),
    ];
    for (field, repositories, child_work, maximum) in invalid {
        let error = MachineConcurrency::new(repositories, child_work).unwrap_err();
        let value = if field == "max_repositories" {
            repositories
        } else {
            child_work
        };
        assert_eq!(
            error.to_string(),
            format!("invalid {field}={value}; expected an integer in 1..={maximum}")
        );
    }
}

#[test]
fn machine_configuration_table_covers_empty_local_and_remote_authority() {
    assert!(
        MachineConfiguration::new(
            SchemaVersion::current(),
            Vec::new(),
            Vec::new(),
            None,
            MachineConcurrency::default(),
            RepairControls::default(),
        )
        .is_ok()
    );

    let local = MachineConfiguration::new(
        SchemaVersion::current(),
        vec![repo("app")],
        vec![source(
            "local",
            SourceLocation::local(path("/sources/local")),
        )],
        Some(path("/cache")),
        MachineConcurrency::new(1, 1).unwrap(),
        RepairControls::new(vec![AgentKind::Codex], 1).unwrap(),
    )
    .unwrap();
    assert!(local.cache_root().is_some());

    let remote = source(
        "remote",
        SourceLocation::remote("https://example.test/source").unwrap(),
    );
    let missing_cache = MachineConfiguration::new(
        SchemaVersion::current(),
        vec![repo("app")],
        vec![remote.clone()],
        None,
        MachineConcurrency::default(),
        RepairControls::default(),
    )
    .unwrap_err();
    assert_eq!(
        missing_cache.to_string(),
        "remote sources require a machine cache root"
    );

    assert!(
        MachineConfiguration::new(
            SchemaVersion::current(),
            vec![repo("app")],
            vec![remote],
            Some(path("/cache")),
            MachineConcurrency::default(),
            RepairControls::default(),
        )
        .is_ok()
    );
}

#[test]
fn machine_configuration_conflict_tables_cover_repository_and_source_authority() {
    let duplicate_repository_id = MachineConfiguration::new(
        SchemaVersion::current(),
        vec![
            DestinationRepository::new(
                RepositoryId::parse("app").unwrap(),
                path("/fleet/first"),
                [],
            )
            .unwrap(),
            DestinationRepository::new(
                RepositoryId::parse("app").unwrap(),
                path("/fleet/second"),
                [],
            )
            .unwrap(),
        ],
        Vec::new(),
        None,
        MachineConcurrency::default(),
        RepairControls::default(),
    )
    .unwrap_err();
    assert_eq!(
        duplicate_repository_id.to_string(),
        "duplicate repository id \"app\" at \"/fleet/first\" and \"/fleet/second\""
    );

    let duplicate_repository_path = MachineConfiguration::new(
        SchemaVersion::current(),
        vec![
            DestinationRepository::new(
                RepositoryId::parse("first").unwrap(),
                path("/fleet/same"),
                [],
            )
            .unwrap(),
            DestinationRepository::new(
                RepositoryId::parse("second").unwrap(),
                path("/fleet/same"),
                [],
            )
            .unwrap(),
        ],
        Vec::new(),
        None,
        MachineConcurrency::default(),
        RepairControls::default(),
    )
    .unwrap_err();
    assert_eq!(
        duplicate_repository_path.to_string(),
        "duplicate destination repository path \"/fleet/same\""
    );

    let duplicate_source_id = MachineConfiguration::new(
        SchemaVersion::current(),
        vec![repo("app")],
        vec![
            source("shared", SourceLocation::local(path("/sources/one"))),
            source("shared", SourceLocation::local(path("/sources/two"))),
        ],
        None,
        MachineConcurrency::default(),
        RepairControls::default(),
    )
    .unwrap_err();
    assert_eq!(
        duplicate_source_id.to_string(),
        "duplicate source id \"shared\""
    );

    let duplicate_source_location = MachineConfiguration::new(
        SchemaVersion::current(),
        vec![repo("app")],
        vec![
            source("first", SourceLocation::local(path("/sources/shared"))),
            source("second", SourceLocation::local(path("/sources/shared"))),
        ],
        None,
        MachineConcurrency::default(),
        RepairControls::default(),
    )
    .unwrap_err();
    assert_eq!(
        duplicate_source_location.to_string(),
        "source ids \"first\" and \"second\" use the same location"
    );
}

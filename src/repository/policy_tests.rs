use std::fs;

use super::policy::*;

fn item(id: &str) -> ManagedItemId {
    ManagedItemId::new(id).expect("fixture uses a valid managed-item ID")
}

fn command(args: &[&str]) -> VerificationCommand {
    VerificationCommand::new(args.iter().copied()).expect("fixture command has argv")
}

#[test]
fn true_absence_is_not_a_present_policy() {
    let state = RepositoryPolicyState::absent();

    assert!(state.is_absent(), "missing policy must be explicit absence");
    assert!(
        !state.is_present(),
        "absence must not look like an empty policy"
    );
    assert!(state.as_snapshot().is_none(), "absence has no snapshot");
    assert!(state.error().is_none(), "absence is not an invalid policy");
}

#[test]
fn explicit_empty_policy_remains_present_and_selects_nothing() {
    let policy = RepositoryPolicy::new(
        SchemaVersion::V1,
        SelectionPolicy::explicit(false, [], []).expect("empty selector is valid"),
        CommandPolicy::absent(),
    )
    .expect("an explicit empty selector is valid");
    let state = RepositoryPolicyState::present(PolicyIdentity::from_bytes([1; 32]), policy);

    let snapshot = state.as_snapshot().expect("present policy has a snapshot");
    assert_eq!(snapshot.identity().as_bytes(), [1; 32]);
    assert!(
        state.is_present(),
        "explicit empty configuration is intentional"
    );
    assert!(!state.is_absent(), "present empty is not true absence");
    assert!(snapshot.policy().selection().selects_nothing());
    assert!(snapshot.policy().commands().is_absent());
    assert_eq!(snapshot.policy().schema_version(), SchemaVersion::V1);
}

#[test]
fn omitted_selector_and_commands_only_are_distinct_present_states() {
    let omitted = RepositoryPolicy::new(
        SchemaVersion::V1,
        SelectionPolicy::omitted(),
        CommandPolicy::absent(),
    )
    .expect("omitted selector is valid");
    assert!(omitted.selection().is_omitted());
    assert!(omitted.selection().selects_nothing());

    let commands_only = RepositoryPolicy::new(
        SchemaVersion::V1,
        SelectionPolicy::omitted(),
        CommandPolicy::present(vec![command(&["cargo", "test"])]).expect("unique command"),
    )
    .expect("commands-only policy is valid");
    assert!(commands_only.selection().is_omitted());
    assert!(commands_only.selection().selects_nothing());
    let commands = commands_only
        .commands()
        .as_slice()
        .expect("commands-only state retains its command list");
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].argv(), ["cargo", "test"]);

    for policy in [omitted, commands_only] {
        let state = RepositoryPolicyState::present(PolicyIdentity::from_bytes([2; 32]), policy);
        assert!(
            state.is_present(),
            "commands-only/omitted policy is present"
        );
        assert!(!state.is_absent(), "presence must not trigger inference");
    }
}

#[test]
fn all_allow_and_exclude_keep_decided_order_and_exclusion_policy() {
    struct SelectorCase {
        name: &'static str,
        all: bool,
        allow: &'static [&'static str],
        exclude: &'static [&'static str],
        selects_nothing: bool,
    }

    let cases = [
        SelectorCase {
            name: "explicit empty",
            all: false,
            allow: &[],
            exclude: &[],
            selects_nothing: true,
        },
        SelectorCase {
            name: "allow only",
            all: false,
            allow: &["docs", "build"],
            exclude: &[],
            selects_nothing: false,
        },
        SelectorCase {
            name: "all",
            all: true,
            allow: &[],
            exclude: &[],
            selects_nothing: false,
        },
        SelectorCase {
            name: "all with an exclusion",
            all: true,
            allow: &["docs", "build"],
            exclude: &["build", "private"],
            selects_nothing: false,
        },
    ];

    for case in cases {
        let selection = SelectionPolicy::explicit(
            case.all,
            case.allow.iter().copied().map(item),
            case.exclude.iter().copied().map(item),
        )
        .expect("valid explicit selector");

        assert_eq!(selection.all(), case.all, "{}: all", case.name);
        assert_eq!(
            selection
                .allow()
                .iter()
                .map(ManagedItemId::as_str)
                .collect::<Vec<_>>(),
            case.allow,
            "{}: allow order",
            case.name
        );
        assert_eq!(
            selection
                .exclude()
                .iter()
                .map(ManagedItemId::as_str)
                .collect::<Vec<_>>(),
            case.exclude,
            "{}: exclude order",
            case.name
        );
        assert_eq!(
            selection.selects_nothing(),
            case.selects_nothing,
            "{}: selection emptiness",
            case.name
        );
    }

    let omitted = SelectionPolicy::omitted();
    assert!(omitted.is_omitted());
    assert!(!omitted.all());
    assert!(omitted.allow().is_empty());
    assert!(omitted.exclude().is_empty());
    assert!(omitted.selects_nothing());
}

#[test]
fn command_snapshot_preserves_argv_order_and_distinguishes_empty_commands() {
    let empty = CommandPolicy::present(Vec::<VerificationCommand>::new())
        .expect("an explicitly empty command list is valid");
    assert!(empty.is_present());
    assert_eq!(
        empty.as_slice().expect("present empty command list").len(),
        0
    );

    let ordered = CommandPolicy::present(vec![
        command(&["first", "--one"]),
        command(&["second"]),
        command(&["third", "--three", "value"]),
    ])
    .expect("ordered commands are unique");
    let commands = ordered.as_slice().expect("commands are present");
    assert_eq!(commands[0].argv(), ["first", "--one"]);
    assert_eq!(commands[1].argv(), ["second"]);
    assert_eq!(commands[2].argv(), ["third", "--three", "value"]);
}

#[test]
fn invalid_policy_is_not_absence_and_keeps_typed_diagnostic() {
    let errors = [
        PolicyError::InvalidManagedItemId {
            value: "../secret".into(),
        },
        PolicyError::DuplicateSelector {
            field: "exclude",
            id: item("docs"),
        },
        PolicyError::EmptyCommandExecutable,
        PolicyError::NulInCommandArgument { index: 1 },
        PolicyError::DuplicateCommand {
            first: 0,
            duplicate: 2,
        },
        PolicyError::UnsupportedSchemaVersion {
            found: 2,
            supported: SchemaVersion::V1,
        },
    ];

    for error in errors {
        let state = RepositoryPolicyState::invalid(error.clone());

        assert!(!state.is_absent(), "{error:?} must not become absence");
        assert!(!state.is_present(), "{error:?} is not a valid snapshot");
        assert!(
            state.as_snapshot().is_none(),
            "{error:?} must not expose a snapshot"
        );
        assert_eq!(state.error(), Some(&error));
        assert!(state.snapshot_error().is_none());
    }
}

#[test]
fn changed_snapshot_is_distinct_from_invalid_and_absent() {
    let policy = RepositoryPolicy::new(
        SchemaVersion::V1,
        SelectionPolicy::omitted(),
        CommandPolicy::absent(),
    )
    .expect("valid policy");
    let expected = PolicyIdentity::from_bytes([3; 32]);
    let observed = PolicyIdentity::from_bytes([4; 32]);
    let snapshot = RepositoryPolicyState::present(expected, policy)
        .as_snapshot()
        .expect("snapshot exists")
        .clone();

    let error = snapshot
        .revalidate(observed)
        .expect_err("replacement must invalidate a frozen policy snapshot");
    assert_eq!(error, PolicySnapshotError::Changed { expected, observed });
    let state = RepositoryPolicyState::changed(error);
    assert!(!state.is_absent());
    assert!(!state.is_present());
    assert!(state.as_snapshot().is_none());
    assert_eq!(state.snapshot_error(), Some(&error));
    assert!(state.error().is_none());
}

#[test]
fn unchanged_snapshot_revalidates_without_effects() {
    let identity = PolicyIdentity::from_bytes([5; 32]);
    let policy = RepositoryPolicy::new(
        SchemaVersion::V1,
        SelectionPolicy::omitted(),
        CommandPolicy::absent(),
    )
    .expect("valid policy");
    let snapshot = RepositoryPolicyState::present(identity, policy)
        .as_snapshot()
        .expect("snapshot exists")
        .clone();

    assert_eq!(snapshot.revalidate(identity), Ok(()));
}

#[test]
fn managed_item_ids_are_exact_lowercase_slugs() {
    for valid in ["all", "docs.v1", "build_cache", "a-b", "a1"] {
        assert!(
            ManagedItemId::new(valid).is_ok(),
            "{valid:?} should be valid"
        );
    }
    for invalid in [
        "",
        "UPPER",
        "space value",
        "slash/name",
        "../secret",
        "a+b",
        "é",
        "line\nbreak",
    ] {
        assert!(
            ManagedItemId::new(invalid).is_err(),
            "{invalid:?} should be rejected as a policy ID"
        );
    }
    assert_eq!(ManagedItemId::parse("docs").unwrap().as_str(), "docs");
}

#[test]
fn empty_verification_argv_is_invalid_without_creating_absence() {
    assert_eq!(
        VerificationCommand::new(std::iter::empty::<&str>()),
        Err(PolicyError::EmptyCommandExecutable)
    );
}

#[test]
fn duplicate_selectors_and_unsupported_versions_are_typed_validation_errors() {
    for (field, allow, exclude) in [
        ("allow", vec![item("docs"), item("docs")], vec![]),
        ("exclude", vec![], vec![item("docs"), item("docs")]),
    ] {
        assert_eq!(
            SelectionPolicy::explicit(false, allow, exclude),
            Err(PolicyError::DuplicateSelector {
                field,
                id: item("docs"),
            })
        );
    }

    assert_eq!(SchemaVersion::current(), SchemaVersion::V1);
    assert_eq!(SchemaVersion::new(1), Ok(SchemaVersion::V1));
    assert_eq!(
        [0, 2, u64::MAX]
            .into_iter()
            .map(SchemaVersion::new)
            .collect::<Vec<_>>(),
        vec![
            Err(PolicyError::UnsupportedSchemaVersion {
                found: 0,
                supported: SchemaVersion::V1,
            }),
            Err(PolicyError::UnsupportedSchemaVersion {
                found: 2,
                supported: SchemaVersion::V1,
            }),
            Err(PolicyError::UnsupportedSchemaVersion {
                found: u64::MAX,
                supported: SchemaVersion::V1,
            }),
        ]
    );
}

#[test]
fn command_domain_rejects_empty_executable_and_nul_in_any_argument() {
    let cases = [
        (vec![""], PolicyError::EmptyCommandExecutable),
        (
            vec!["cargo\0test"],
            PolicyError::NulInCommandArgument { index: 0 },
        ),
        (
            vec!["cargo", "secret-token\0--locked"],
            PolicyError::NulInCommandArgument { index: 1 },
        ),
    ];

    for (argv, expected) in cases {
        let error = VerificationCommand::new(argv).expect_err("hostile argv must be rejected");
        assert_eq!(error, expected);
        let rendered = error.to_string();
        assert_eq!(rendered, expected.to_string());
        assert!(
            !rendered.contains("secret-token"),
            "contextual diagnostics must not echo command arguments"
        );
    }
}

#[test]
fn command_domain_rejects_exact_duplicate_argv_but_preserves_distinct_order() {
    let duplicate = vec![command(&["cargo", "test"]), command(&["cargo", "test"])];
    let error = CommandPolicy::present(duplicate).expect_err("duplicate argv must be rejected");
    assert_eq!(
        error,
        PolicyError::DuplicateCommand {
            first: 0,
            duplicate: 1
        }
    );
    assert_eq!(
        error.to_string(),
        "verification command 1 duplicates command 0 exactly"
    );
    assert!(
        !error.to_string().contains("cargo"),
        "duplicate diagnostics must not echo command argv"
    );

    let distinct = CommandPolicy::present(vec![
        command(&["cargo", "test"]),
        command(&["cargo", "test", "--locked"]),
        command(&["cargo", "check"]),
    ])
    .expect("distinct command argv is valid");
    let commands = distinct.as_slice().expect("commands are present");
    assert_eq!(commands[0].argv(), ["cargo", "test"]);
    assert_eq!(commands[1].argv(), ["cargo", "test", "--locked"]);
    assert_eq!(commands[2].argv(), ["cargo", "check"]);
}

#[test]
fn policy_error_messages_are_stable_and_contextual() {
    let cases = [
        (
            PolicyError::InvalidManagedItemId {
                value: "../secret".into(),
            },
            "invalid managed-item ID \"../secret\"",
        ),
        (
            PolicyError::DuplicateSelector {
                field: "exclude",
                id: item("docs"),
            },
            "duplicate exclude selector ManagedItemId(\"docs\")",
        ),
        (
            PolicyError::EmptyCommandExecutable,
            "verification command executable cannot be empty",
        ),
        (
            PolicyError::NulInCommandArgument { index: 3 },
            "verification command argument 3 contains NUL",
        ),
        (
            PolicyError::DuplicateCommand {
                first: 0,
                duplicate: 2,
            },
            "verification command 2 duplicates command 0 exactly",
        ),
        (
            PolicyError::UnsupportedSchemaVersion {
                found: 7,
                supported: SchemaVersion::V1,
            },
            "unsupported repository policy schema version 7; expected 1",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn snapshot_change_diagnostic_names_only_authority_identities() {
    let error = PolicySnapshotError::Changed {
        expected: PolicyIdentity::from_bytes([1; 32]),
        observed: PolicyIdentity::from_bytes([2; 32]),
    };
    let rendered = error.to_string();

    assert!(rendered.contains("repository policy changed during snapshot"));
    assert!(rendered.contains(&format!("{:?}", [1; 32])));
    assert!(rendered.contains(&format!("{:?}", [2; 32])));
    assert!(
        !rendered.contains("cargo") && !rendered.contains("source") && !rendered.contains("fleet"),
        "snapshot diagnostics must not expose policy or authority content"
    );
}

#[test]
fn policy_selection_and_commands_have_no_public_unvalidated_enum_variants() {
    let source = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/repository/policy.rs"
    ))
    .expect("read repository policy domain source");

    assert!(
        !source.contains("pub enum SelectionPolicy"),
        "selection policy must be opaque and constructor-validated"
    );
    assert!(
        !source.contains("pub enum CommandPolicy"),
        "command policy must be opaque and constructor-validated"
    );

    for forbidden_import in [
        "std::env",
        "std::fs",
        "std::io",
        "std::net",
        "std::path",
        "std::process",
        "std::thread",
        "Command::",
    ] {
        assert!(
            !source.contains(forbidden_import),
            "policy domain must not perform external effects through {forbidden_import}"
        );
    }

    let fields = source
        .split_once("pub struct RepositoryPolicy")
        .and_then(|(_, remainder)| remainder.split_once("impl RepositoryPolicy"))
        .map(|(fields, _)| fields)
        .expect("repository policy has a visible struct boundary");
    for foreign_authority in ["fleet", "machine", "priority", "source", "source_order"] {
        assert!(
            !fields.contains(foreign_authority),
            "repository policy must not own {foreign_authority} authority"
        );
    }
}

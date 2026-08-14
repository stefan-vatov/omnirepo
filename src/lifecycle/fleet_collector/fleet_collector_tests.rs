//! Focused proof for collecting one initial result per fleet member.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::fleet_collector::{CollectError, MemberResult, collect_fleet_results};
use crate::lifecycle::fleet_fanout::RepoResult;

#[test]
fn every_declared_fleet_member_receives_exactly_one_result() {
    let results = vec![
        RepoResult::Delivered {
            repository: "dest-a".to_owned(),
            oid: "oid-a".to_owned(),
        },
        RepoResult::Failed {
            repository: "dest-b".to_owned(),
            reason: "verify".to_owned(),
        },
        RepoResult::Skipped {
            repository: "dest-c".to_owned(),
            reason: "preflight".to_owned(),
        },
    ];
    let fleet = collect_fleet_results(
        results,
        &[
            "dest-a".to_owned(),
            "dest-b".to_owned(),
            "dest-c".to_owned(),
        ],
    )
    .expect("collect");
    assert_eq!(fleet.members.len(), 3, "one result per member");
    assert!(matches!(fleet.members[0], MemberResult::Delivered { .. }));
    assert!(matches!(fleet.members[1], MemberResult::Failed { .. }));
    assert!(matches!(fleet.members[2], MemberResult::Skipped { .. }));
}

#[test]
fn missing_members_fail_typed() {
    let results = vec![RepoResult::Delivered {
        repository: "dest-a".to_owned(),
        oid: "oid-a".to_owned(),
    }];
    let error = collect_fleet_results(results, &["dest-a".to_owned(), "dest-b".to_owned()])
        .expect_err("missing member");
    assert!(
        matches!(error, CollectError::MissingMember { .. }),
        "{error}"
    );
}

#[test]
fn duplicate_results_fail_typed() {
    let results = vec![
        RepoResult::Delivered {
            repository: "dest-a".to_owned(),
            oid: "oid-a".to_owned(),
        },
        RepoResult::Delivered {
            repository: "dest-a".to_owned(),
            oid: "oid-b".to_owned(),
        },
    ];
    let error = collect_fleet_results(results, &["dest-a".to_owned()]).expect_err("duplicate");
    assert!(
        matches!(error, CollectError::DuplicateResult { .. }),
        "{error}"
    );
}

#[test]
fn declared_order_is_preserved() {
    let results = vec![
        RepoResult::Delivered {
            repository: "dest-z".to_owned(),
            oid: "z".to_owned(),
        },
        RepoResult::Delivered {
            repository: "dest-a".to_owned(),
            oid: "a".to_owned(),
        },
    ];
    let fleet = collect_fleet_results(results, &["dest-z".to_owned(), "dest-a".to_owned()])
        .expect("collect");
    let ids: Vec<&str> = fleet
        .members
        .iter()
        .map(|member| match member {
            MemberResult::Delivered { repository, .. }
            | MemberResult::Failed { repository, .. }
            | MemberResult::Skipped { repository, .. } => repository.as_str(),
        })
        .collect();
    assert_eq!(ids, vec!["dest-z", "dest-a"]);
}

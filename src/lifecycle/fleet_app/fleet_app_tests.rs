//! Focused proof for the internal initial fleet application request and
//! response.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::fleet_app::{FleetRequest, FleetResponse, build_request, finalize_response};
use crate::lifecycle::fleet_collector::MemberResult;

#[test]
fn request_accepts_no_url_file_template_or_destination_override() {
    let request = build_request(&["dest-a".to_owned(), "dest-b".to_owned()], "machine-1", 4)
        .expect("request");
    // The request is internal-only: no URL, no local file, no template,
    // no destination override fields exist.
    let _: &FleetRequest = &request;
    assert_eq!(request.fleet.len(), 2);
    assert_eq!(request.machine_identity, "machine-1");
    assert_eq!(request.limit, 4);
}

#[test]
fn empty_or_zero_limits_fail_typed() {
    assert!(build_request(&[], "machine-1", 4).is_err(), "empty fleet");
    assert!(
        build_request(&["dest-a".to_owned()], "machine-1", 0).is_err(),
        "zero limit"
    );
}

#[test]
fn response_contains_every_initial_result_and_frozen_repair_inputs() {
    let members = vec![
        MemberResult::Delivered {
            repository: "dest-a".to_owned(),
            oid: "oid-a".to_owned(),
        },
        MemberResult::Failed {
            repository: "dest-b".to_owned(),
            reason: "verify failed".to_owned(),
        },
    ];
    let response = finalize_response(
        "run-1",
        members,
        vec![("dest-b".to_owned(), "repair-candidate".to_owned())],
    )
    .expect("response");
    assert_eq!(response.results.len(), 2, "every initial result present");
    assert_eq!(
        response.frozen_repair_inputs.len(),
        1,
        "frozen repair inputs"
    );
    assert_eq!(response.frozen_repair_inputs[0].0, "dest-b");
    assert_eq!(response.frozen_repair_inputs[0].1, "repair-candidate");
    let _: &FleetResponse = &response;
}

#[test]
fn response_members_and_repair_inputs_are_consistent() {
    // Every repair input names a member that actually failed.
    let members = vec![
        MemberResult::Delivered {
            repository: "dest-a".to_owned(),
            oid: "oid-a".to_owned(),
        },
        MemberResult::Failed {
            repository: "dest-b".to_owned(),
            reason: "verify failed".to_owned(),
        },
    ];
    let response = finalize_response(
        "run-1",
        members.clone(),
        vec![("dest-b".to_owned(), "repair-candidate".to_owned())],
    )
    .expect("response");
    for (repository, _) in &response.frozen_repair_inputs {
        assert!(
            members
                .iter()
                .any(|member| matches!(member, MemberResult::Failed { repository: id, .. } if id == repository)),
            "repair input {repository} has no failed member"
        );
    }
}

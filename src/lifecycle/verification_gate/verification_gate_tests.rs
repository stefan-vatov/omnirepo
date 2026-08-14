//! Focused proof for the post-verification gate.

#![allow(dead_code, unused_imports)]

use super::{GateInputs, GateVerdict, account_cleanup, revalidate_gate};
use crate::lifecycle::verifier_confinement::ArtifactDisposition;
use crate::managed_content::CompareOutcome;

fn inputs() -> GateInputs {
    GateInputs {
        frozen_plan_identity: "plan-1".to_owned(),
        observed_plan_identity: "plan-1".to_owned(),
        frozen_source_identity: "source-1".to_owned(),
        observed_source_identity: "source-1".to_owned(),
        frozen_configuration_identity: "config-1".to_owned(),
        observed_configuration_identity: "config-1".to_owned(),
    }
}

#[test]
fn drift_and_forbidden_delta_convert_verification_to_failure() {
    assert_eq!(
        revalidate_gate(&inputs(), CompareOutcome::Unchanged, false, false),
        GateVerdict::ForbiddenDelta
    );
    let drifted = GateInputs {
        frozen_plan_identity: "plan-1".to_owned(),
        observed_plan_identity: "plan-1".to_owned(),
        frozen_source_identity: "source-1".to_owned(),
        observed_source_identity: "source-1".to_owned(),
        frozen_configuration_identity: "config-1".to_owned(),
        observed_configuration_identity: "config-1".to_owned(),
    };
    // A replacement outcome means the target drifted from the frozen
    // bytes.
    assert_ne!(
        revalidate_gate(
            &drifted,
            crate::managed_content::CompareOutcome::Replacement(dummy_plan()),
            true,
            false,
        ),
        GateVerdict::Pass
    );
}

#[test]
fn identity_changes_and_concurrent_modification_prevent_git() {
    let changed = GateInputs {
        frozen_plan_identity: "plan-1".to_owned(),
        observed_plan_identity: "plan-2".to_owned(),
        frozen_source_identity: "source-1".to_owned(),
        observed_source_identity: "source-1".to_owned(),
        frozen_configuration_identity: "config-1".to_owned(),
        observed_configuration_identity: "config-1".to_owned(),
    };
    assert_eq!(
        revalidate_gate(&changed, CompareOutcome::Unchanged, true, false),
        GateVerdict::IdentityChanged
    );
    assert_eq!(
        revalidate_gate(&inputs(), CompareOutcome::Unchanged, true, true),
        GateVerdict::ConcurrentModification
    );
}

#[test]
fn pass_requires_parity_identity_and_authorization_together() {
    assert_eq!(
        revalidate_gate(&inputs(), CompareOutcome::Unchanged, true, false),
        GateVerdict::Pass
    );
    // Any single broken input prevents the pass.
    let broken_source = GateInputs {
        frozen_plan_identity: "plan-1".to_owned(),
        observed_plan_identity: "plan-1".to_owned(),
        frozen_source_identity: "source-1".to_owned(),
        observed_source_identity: "source-2".to_owned(),
        frozen_configuration_identity: "config-1".to_owned(),
        observed_configuration_identity: "config-1".to_owned(),
    };
    assert_eq!(
        revalidate_gate(&broken_source, CompareOutcome::Unchanged, true, false),
        GateVerdict::IdentityChanged
    );
}

#[test]
fn ephemeral_cleanup_is_accounted() {
    assert!(
        account_cleanup(&ArtifactDisposition::Cleaned {
            path: "a.txt".to_owned()
        })
        .is_ok()
    );
    assert!(
        account_cleanup(&ArtifactDisposition::Retained {
            path: "a.txt".to_owned()
        })
        .is_ok()
    );
}

fn dummy_plan() -> crate::managed_content::TransactionPlan {
    crate::managed_content::TransactionPlan::new(
        "run-1",
        std::path::PathBuf::from("managed.txt"),
        crate::managed_content::ParentDirectories::existing(),
    )
    .expect("plan")
}

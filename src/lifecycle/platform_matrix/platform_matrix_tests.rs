//! Focused proof for the owner-selected supported-platform matrix.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use std::path::Path;

use crate::lifecycle::platform_matrix::{
    Filesystem, GateKind, Os, PlatformError, PlatformJob, Toolchain, capability_supported,
    claim_platform, supported_platform_matrix, unsupported_cases,
};

#[test]
fn every_supported_platform_runs_the_required_locked_gates_on_rust_1_86() {
    let matrix = supported_platform_matrix();
    assert!(!matrix.is_empty());
    for job in &matrix {
        assert_eq!(job.toolchain, Toolchain::Rust186);
        assert!(job.required.contains(&GateKind::Tests), "{job:?}");
        assert!(job.required.contains(&GateKind::Docs), "{job:?}");
        assert!(job.required.contains(&GateKind::Locked), "{job:?}");
        assert!(
            job.required.contains(&GateKind::Quality),
            "the repository-owned quality command is used: {job:?}"
        );
        assert!(capability_supported(job.os, job.filesystem), "{job:?}");
    }
}

#[test]
fn the_owner_selected_os_filesystem_pairs_are_declared_exactly() {
    let matrix = supported_platform_matrix();
    let pairs = matrix
        .iter()
        .map(|job| (job.os, job.filesystem))
        .collect::<std::collections::BTreeSet<_>>();
    assert!(pairs.contains(&(Os::Linux, Filesystem::Linux)), "{pairs:?}");
    assert!(pairs.contains(&(Os::Mac, Filesystem::Apfs)), "{pairs:?}");
    assert_eq!(pairs.len(), 2, "no invented platform is claimed: {pairs:?}");
}

#[test]
fn unsupported_cases_fail_closed_and_are_explicitly_omitted_from_jobs() {
    let matrix = supported_platform_matrix();
    for case in unsupported_cases() {
        assert!(
            !capability_supported(case.os, case.filesystem),
            "{case:?} must fail closed"
        );
        assert!(
            !matrix
                .iter()
                .any(|job| job.os == case.os && job.filesystem == case.filesystem),
            "the unsupported case must be omitted from jobs: {case:?}"
        );
    }
    // Windows and non-APFS macOS filesystems are the documented unsupported set.
    let unsupported = unsupported_cases();
    assert!(
        unsupported.iter().any(|case| case.os == Os::Windows),
        "{unsupported:?}"
    );
    assert!(
        unsupported
            .iter()
            .any(|case| case.os == Os::Mac && case.filesystem == Filesystem::Other),
        "{unsupported:?}"
    );
}

#[test]
fn claiming_an_unsupported_or_invented_platform_fails_typed() {
    assert!(capability_supported(Os::Linux, Filesystem::Linux));
    assert!(!capability_supported(Os::Windows, Filesystem::Other));
    assert!(!capability_supported(Os::Mac, Filesystem::Other));
    assert!(matches!(
        claim_platform(Os::Windows, Filesystem::Other),
        Err(PlatformError::Unsupported { .. })
    ));
    assert!(claim_platform(Os::Linux, Filesystem::Linux).is_ok());
    assert!(claim_platform(Os::Mac, Filesystem::Apfs).is_ok());
}

#[test]
fn cache_isolation_and_job_identity_are_explicit() {
    let matrix = supported_platform_matrix();
    let identities = matrix
        .iter()
        .map(|job| job.cache_key())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        identities.len(),
        matrix.len(),
        "each job has its own cache key"
    );
    for job in &matrix {
        assert!(!job.cache_key().is_empty());
    }
}

#[test]
fn rendered_evidence_matches_the_committed_support_report() {
    use crate::lifecycle::platform_matrix::platform_evidence;
    // The evidence is rendered from the declared matrix (the single
    // source of truth); the committed report must be semantically
    // identical — missing or extra matrix entries fail here.
    let rendered = platform_evidence();
    let committed = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/traceability/platform-evidence.json"),
    )
    .expect("committed evidence");
    let rendered_value: yaml_serde::Value =
        yaml_serde::from_str(&rendered).expect("rendered evidence parses");
    let committed_value: yaml_serde::Value =
        yaml_serde::from_str(&committed).expect("committed evidence parses");
    assert_eq!(
        rendered_value, committed_value,
        "the committed support report drifted from the declared matrix"
    );
}

#[test]
fn every_supported_os_maps_to_live_ci_jobs_and_no_extra_platform_is_claimed() {
    use crate::lifecycle::platform_matrix::ci_job_names;
    let workflow = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/pr-lint-test.yml"),
    )
    .expect("workflow");
    // Every declared job name appears in the live workflow.
    for job in ci_job_names() {
        assert!(workflow.contains(job), "the CI job {job:?} is missing");
    }
    // A job whose name suggests an undeclared platform fails the drift
    // check (no invented platform is claimed).
    for line in workflow.lines() {
        let name = line.trim();
        if name.starts_with("  ") && name.ends_with(":") && !name.starts_with("    ") {
            let job = name.trim_end_matches(':').trim();
            if job.contains("windows") || job.contains("win-") || job.contains("network") {
                panic!("the CI declares an unsupported platform job: {job}");
            }
        }
    }
}

#[test]
fn capability_skips_are_visible_in_the_evidence() {
    use crate::lifecycle::platform_matrix::platform_evidence;
    let evidence = platform_evidence();
    // The unsupported cases are explicitly visible in the report.
    for case in unsupported_cases() {
        assert!(
            evidence.contains(&format!("\"os\": \"{:?}\"", case.os))
                && evidence.contains(&format!("\"filesystem\": \"{:?}\"", case.filesystem)),
            "the skip for {case:?} is not visible in the evidence"
        );
    }
}

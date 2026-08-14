//! Focused proof for reapplying authoritative synchronization and rerunning
//! the frozen verification after a successful repair.
//!
//! STRICT TDD: this test file was written and run RED before the
//! implementation existed.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::repair_reapply::{
    ReapplyError, ReapplyVerdict, reapply_authoritative, rerun_frozen_verification,
};
use std::{fs, path::Path};

fn fixture_root() -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix("repair-reapply-")
        .tempdir_in(&base)
        .expect("fixture")
}

#[test]
fn after_reapply_the_frozen_verification_passes() {
    let fixture = fixture_root();
    fs::write(fixture.path().join("managed.md"), "v1\n").expect("file");
    let authoritative = b"authoritative-v2\n";
    reapply_authoritative(fixture.path(), "managed.md", authoritative).expect("reapply");
    let bytes = fs::read(fixture.path().join("managed.md")).expect("read");
    let text = String::from_utf8(bytes).expect("utf8");
    assert!(
        text.contains("<!-- omnirepo-start -->") && text.contains("authoritative-v2"),
        "the authoritative payload is delivered inside the managed section: {text:?}"
    );
    assert!(matches!(
        rerun_frozen_verification(
            fixture.path(),
            "managed.md",
            authoritative,
            &["baseline-1".to_owned()],
        ),
        Ok(ReapplyVerdict::Verified)
    ));
}

#[test]
fn a_drifted_destination_fails_the_frozen_verification() {
    let fixture = fixture_root();
    fs::write(fixture.path().join("managed.md"), "drifted\n").expect("file");
    let authoritative = b"authoritative-v2\n";
    assert!(matches!(
        rerun_frozen_verification(
            fixture.path(),
            "managed.md",
            authoritative,
            &["baseline-1".to_owned()],
        ),
        Ok(ReapplyVerdict::Drifted)
    ));
}

#[test]
fn empty_frozen_inputs_fail_the_frozen_verification() {
    let fixture = fixture_root();
    fs::write(fixture.path().join("managed.md"), "x\n").expect("file");
    let authoritative = b"x\n";
    assert!(matches!(
        rerun_frozen_verification(fixture.path(), "managed.md", authoritative, &[]),
        Err(ReapplyError::FrozenInputsMissing)
    ));
}

#[test]
fn a_missing_managed_file_fails_typed() {
    let fixture = fixture_root();
    let error = rerun_frozen_verification(
        fixture.path(),
        "absent.md",
        b"x\n",
        &["baseline-1".to_owned()],
    )
    .expect_err("missing");
    assert!(
        matches!(
            error,
            ReapplyError::Read { .. } | ReapplyError::ManagedPath { .. }
        ),
        "{error}"
    );
}

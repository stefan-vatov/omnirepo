//! Focused proof for reapplying authoritative synchronization and rerunning
//! the frozen verification after a successful repair.

#![allow(dead_code, unused_imports)]

use crate::configuration::SectionId;
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

fn section(id: &str) -> SectionId {
    SectionId::new(id).expect("valid id")
}

#[test]
fn after_reapply_the_frozen_verification_passes() {
    let fixture = fixture_root();
    fs::write(
        fixture.path().join("managed.md"),
        "local\n\n<!-- omnirepo:start rules -->\nv1\n<!-- omnirepo:end rules -->\n",
    )
    .expect("file");
    let authoritative = b"authoritative-v2\n";
    let rules = section("rules");
    reapply_authoritative(fixture.path(), "managed.md", Some(&rules), authoritative)
        .expect("reapply");
    let bytes = fs::read(fixture.path().join("managed.md")).expect("read");
    let text = String::from_utf8(bytes).expect("utf8");
    assert!(
        text.starts_with("local\n")
            && text.contains("<!-- omnirepo:start rules -->")
            && text.contains("authoritative-v2"),
        "the authoritative payload is delivered inside the named section and local content survives: {text:?}"
    );
    assert!(matches!(
        rerun_frozen_verification(
            fixture.path(),
            "managed.md",
            Some(&rules),
            authoritative,
            &["baseline-1".to_owned()],
        ),
        Ok(ReapplyVerdict::Verified)
    ));
}

#[test]
fn reapply_preserves_other_named_sections() {
    let fixture = fixture_root();
    fs::write(
        fixture.path().join("managed.md"),
        "<!-- omnirepo:start alpha -->\na\n<!-- omnirepo:end alpha -->\n\n<!-- omnirepo:start beta -->\nb\n<!-- omnirepo:end beta -->\n",
    )
    .expect("file");
    let alpha = section("alpha");
    reapply_authoritative(fixture.path(), "managed.md", Some(&alpha), b"a2\n").expect("reapply");
    let text = String::from_utf8(fs::read(fixture.path().join("managed.md")).expect("read"))
        .expect("utf8");
    assert!(text.contains("a2\n"), "{text:?}");
    assert!(
        text.contains("<!-- omnirepo:start beta -->\nb\n<!-- omnirepo:end beta -->"),
        "the untouched section survives byte-exact: {text:?}"
    );
}

#[test]
fn whole_file_reapply_rewrites_byte_exact() {
    let fixture = fixture_root();
    fs::write(fixture.path().join("managed.md"), "drifted\n").expect("file");
    let authoritative = b"authoritative-v2\n";
    reapply_authoritative(fixture.path(), "managed.md", None, authoritative).expect("reapply");
    assert_eq!(
        fs::read(fixture.path().join("managed.md")).expect("read"),
        authoritative
    );
    assert!(matches!(
        rerun_frozen_verification(
            fixture.path(),
            "managed.md",
            None,
            authoritative,
            &["baseline-1".to_owned()],
        ),
        Ok(ReapplyVerdict::Verified)
    ));
}

#[test]
fn reapply_replaces_atomically_without_residue_and_preserves_the_mode() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = fixture_root();
    fs::write(fixture.path().join("managed.md"), "v1\n").expect("file");
    fs::set_permissions(
        fixture.path().join("managed.md"),
        fs::Permissions::from_mode(0o664),
    )
    .expect("mode");
    reapply_authoritative(fixture.path(), "managed.md", None, b"authoritative-v2\n")
        .expect("reapply");
    // The replacement went through the same-directory temporary + rename
    // path: no temporary residue survives a successful reapply, and the
    // existing mode is preserved exactly.
    let residue = fs::read_dir(fixture.path())
        .expect("dir")
        .filter(|entry| {
            entry
                .as_ref()
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .contains("omnirepo-tmp")
        })
        .count();
    assert_eq!(residue, 0, "no temporary residue after success");
    let mode = fs::metadata(fixture.path().join("managed.md"))
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o664, "the existing mode is preserved exactly");
}

#[test]
fn a_drifted_destination_fails_the_frozen_verification() {
    let fixture = fixture_root();
    fs::write(fixture.path().join("managed.md"), "drifted\n").expect("file");
    let authoritative = b"authoritative-v2\n";
    let rules = section("rules");
    // Section mode: the named section is absent entirely — drifted.
    assert!(matches!(
        rerun_frozen_verification(
            fixture.path(),
            "managed.md",
            Some(&rules),
            authoritative,
            &["baseline-1".to_owned()],
        ),
        Ok(ReapplyVerdict::Drifted)
    ));
    // Whole-file mode: different bytes — drifted.
    assert!(matches!(
        rerun_frozen_verification(
            fixture.path(),
            "managed.md",
            None,
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
    assert!(matches!(
        rerun_frozen_verification(fixture.path(), "managed.md", None, b"x\n", &[]),
        Err(ReapplyError::FrozenInputsMissing)
    ));
}

#[test]
fn a_missing_managed_file_fails_typed() {
    let fixture = fixture_root();
    let error = rerun_frozen_verification(
        fixture.path(),
        "absent.md",
        None,
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

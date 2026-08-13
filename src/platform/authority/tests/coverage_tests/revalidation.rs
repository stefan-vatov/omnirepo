//! Deterministic mutation revalidation invariants.

use super::super::{
    AuthorityRoot, DestinationRepositoryRoot, Mutate, MutationIntent, PathError, RelativePath,
};
use crate::platform::authority::backend::{inspect_metadata, reject_unsafe_hard_link};
use std::{
    fs,
    mem::ManuallyDrop,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::Path,
    process::Command,
};
use tempfile::{Builder, TempDir};

const CLOSED_DESCRIPTOR_CHILD: &str = "OMNIREPO_REVALIDATION_CLOSED_DESCRIPTOR_CHILD";

fn run_in_isolated_child(test_name: &str) -> bool {
    if std::env::var_os(CLOSED_DESCRIPTOR_CHILD).is_some() {
        return false;
    }
    let output = Command::new(std::env::current_exe().expect("locate authority test binary"))
        .arg(test_name)
        .arg("--exact")
        .arg("--nocapture")
        .env(CLOSED_DESCRIPTOR_CHILD, "1")
        .output()
        .expect("run isolated authority descriptor test");
    let summary = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && summary.contains("1 passed"),
        "isolated authority test must run exactly one test: {test_name}\n{summary}"
    );
    true
}

#[test]
#[should_panic(expected = "must run exactly one test")]
fn isolated_child_helper_fails_on_wrong_selection() {
    run_in_isolated_child(
        "platform::authority::tests::coverage_tests::revalidation::no_such_descriptor_test",
    );
}

struct ClosedFile {
    file: ManuallyDrop<fs::File>,
}

impl ClosedFile {
    fn new(file: fs::File) -> Self {
        let file = ManuallyDrop::new(file);
        let fd = file.as_raw_fd();
        drop(unsafe { OwnedFd::from_raw_fd(fd) });
        Self { file }
    }

    fn as_file(&self) -> &fs::File {
        &self.file
    }
}

impl Drop for ClosedFile {
    fn drop(&mut self) {
        let replacement = fs::File::open("/").expect("open replacement descriptor");
        let _closed = std::mem::replace(&mut self.file, ManuallyDrop::new(replacement));
        unsafe { ManuallyDrop::drop(&mut self.file) };
    }
}

fn closed_file() -> ClosedFile {
    ClosedFile::new(fs::File::open("/").expect("open descriptor to close"))
}

fn test_directory() -> TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("create authority revalidation fixture base");
    Builder::new()
        .prefix("authority-revalidation-")
        .tempdir_in(base)
        .expect("create authority revalidation fixture")
}

fn mutation_root(path: &Path) -> Option<AuthorityRoot<DestinationRepositoryRoot, Mutate>> {
    match AuthorityRoot::<DestinationRepositoryRoot, Mutate>::open(path) {
        Ok(root) => Some(root),
        Err(PathError::UnsupportedFilesystem { .. }) => None,
        Err(error) => panic!("supported fixture root failed: {error}"),
    }
}

#[test]
fn nested_target_revalidation_preserves_the_resolved_leaf_invariant() {
    let fixture = test_directory();
    let parent = fixture.path().join("parent");
    let target_path = parent.join("managed.txt");
    fs::create_dir(&parent).expect("create authority parent");
    fs::write(&target_path, b"authoritative payload").expect("write authority target");

    let Some(root) = mutation_root(fixture.path()) else {
        return;
    };
    let relative = RelativePath::parse("parent/managed.txt").expect("parse nested target");
    let target = root
        .resolve_mutation(&relative, MutationIntent::Replace)
        .expect("resolve nested mutation target");

    assert_eq!(
        target.relative_path(),
        &relative,
        "revalidation must retain the exact relative target text"
    );
    assert_eq!(
        target.revalidate(),
        Ok(()),
        "the resolver and revalidator must agree on the nested leaf name"
    );
    let file = target
        .into_file()
        .expect("a stable nested target must remain usable");
    assert_eq!(
        file.metadata()
            .expect("inspect the stable nested target")
            .len(),
        b"authoritative payload".len() as u64,
        "revalidation must preserve the originally resolved object"
    );
}

#[test]
fn metadata_revalidation_mapping_reports_closed_descriptor_context() {
    if run_in_isolated_child(
        "platform::authority::tests::coverage_tests::revalidation::metadata_revalidation_mapping_reports_closed_descriptor_context",
    ) {
        return;
    }
    for (operation, path) in [
        ("inspect mutation target", "existing-metadata"),
        ("inspect created mutation target", "created-metadata"),
    ] {
        let file = closed_file();
        let error = inspect_metadata(file.as_file(), operation, path)
            .expect_err("closed descriptor metadata must map to a typed authority error");
        assert!(matches!(
            error,
            PathError::Io {
                operation: actual_operation,
                path: actual_path,
                code: Some(9),
                kind,
            } if actual_operation == operation && actual_path == path && !kind.is_empty()
        ));
    }
}

#[test]
fn hard_link_policy_accepts_single_link_and_rejects_aliases() {
    let fixture = test_directory();
    let target = fixture.path().join("managed");
    let alias = fixture.path().join("managed-alias");
    fs::write(&target, b"authoritative payload").expect("write hard-link fixture");

    let single_link_metadata = fs::metadata(&target).expect("inspect single-link target");
    assert_eq!(
        reject_unsafe_hard_link("managed", &single_link_metadata),
        Ok(()),
        "a single-link target is safe"
    );

    fs::hard_link(&target, &alias).expect("create hard-link alias");
    let aliased_metadata = fs::metadata(&target).expect("inspect aliased target");
    assert_eq!(
        reject_unsafe_hard_link("managed", &aliased_metadata),
        Err(PathError::UnsafeHardLink {
            path: "managed".to_owned(),
            links: 2,
        }),
        "an aliased target is unsafe for mutation"
    );
}

//! Focused authority acceptance, target, adapter, and registry coverage.

use super::capability;

use super::super::{
    AbsolutePath, AuthorityAcceptance, AuthorityAdapter, AuthorityAdapterKind, AuthorityRegistry,
    AuthorityRoot, DestinationRepositoryRoot, MachineConfigRoot, Mutate, MutationIntent, PathError,
    ReadOnly, RelativePath, open_mutation_root, open_read_root,
};
use std::{
    fs,
    io::Read,
    mem::ManuallyDrop,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::{Builder, TempDir};

const CLOSED_READ_HANDLE_CHILD: &str = "OMNIREPO_CLOSED_READ_HANDLE_CHILD";

fn run_in_isolated_child(test_name: &str) -> bool {
    if std::env::var_os(CLOSED_READ_HANDLE_CHILD).is_some() {
        return false;
    }
    let status = Command::new(std::env::current_exe().expect("locate authority test binary"))
        .arg(test_name)
        .arg("--exact")
        .arg("--nocapture")
        .env(CLOSED_READ_HANDLE_CHILD, "1")
        .status()
        .expect("run isolated closed-read-handle test");
    assert!(
        status.success(),
        "isolated closed-read-handle test failed: {test_name}"
    );
    true
}

struct ForwardingAdapter {
    acceptance: AuthorityAcceptance,
}

impl AuthorityAdapter for ForwardingAdapter {
    fn authority_acceptance(&self) -> &AuthorityAcceptance {
        &self.acceptance
    }
}

fn test_directory() -> TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("create deterministic authority fixture base");
    let fixture = Builder::new()
        .prefix("authority-adapter-")
        .tempdir_in(base)
        .expect("create deterministic authority fixture");
    capability::report(fixture.path());
    fixture
}

fn read_root(path: &Path) -> Option<AuthorityRoot<DestinationRepositoryRoot, ReadOnly>> {
    match open_read_root::<DestinationRepositoryRoot>(path) {
        Ok(root) => Some(root),
        Err(PathError::UnsupportedFilesystem { .. }) => None,
        Err(error) => panic!("supported read fixture root failed: {error}"),
    }
}

fn mutation_root(path: &Path) -> Option<AuthorityRoot<DestinationRepositoryRoot, Mutate>> {
    match open_mutation_root::<DestinationRepositoryRoot>(path) {
        Ok(root) => Some(root),
        Err(PathError::UnsupportedFilesystem { .. }) => None,
        Err(error) => panic!("supported mutation fixture root failed: {error}"),
    }
}

fn two_root_fixture() -> (TempDir, PathBuf, PathBuf) {
    let fixture = test_directory();
    let first = fixture.path().join("first");
    let second = fixture.path().join("second");
    fs::create_dir(&first).expect("create first authority root");
    fs::create_dir(&second).expect("create second authority root");
    (fixture, first, second)
}

fn assert_exact_error(case_id: &str, actual: PathError, expected: PathError) {
    assert_eq!(
        actual, expected,
        "[{case_id}] expected exact authority error; expected {expected:?}"
    );
}

#[test]
fn case_8_2_01_acceptance_exposes_owner_root_and_identity_for_matching_targets() {
    let fixture = test_directory();
    let file_path = fixture.path().join("managed.txt");
    fs::write(&file_path, b"authoritative").expect("write managed fixture");
    let Some(read_root) = read_root(fixture.path()) else {
        return;
    };
    let Some(mutation_root) = mutation_root(fixture.path()) else {
        return;
    };
    let path = RelativePath::parse("managed.txt").expect("parse managed path");
    let read_target = read_root
        .resolve_read(&path, super::super::ObjectClass::RegularFile)
        .expect("resolve read target in owning root");
    let mutation_target = mutation_root
        .resolve_mutation(&path, MutationIntent::Replace)
        .expect("resolve mutation target in owning root");

    let acceptance = read_root.acceptance(AuthorityAdapterKind::Configuration);

    assert_eq!(
        acceptance.owner(),
        AuthorityAdapterKind::Configuration,
        "[8.2.01-owner] acceptance owner must remain exact"
    );
    assert_eq!(
        acceptance.root_path().as_path(),
        AbsolutePath::from_path(fixture.path())
            .expect("fixture path is absolute UTF-8")
            .as_path(),
        "[8.2.01-root] acceptance root path must preserve the declared root"
    );
    assert_eq!(
        acceptance.root_identity(),
        read_root.identity(),
        "[8.2.01-identity] acceptance identity must equal its owning root"
    );
    assert_eq!(
        acceptance.accept_read_target(&read_target),
        Ok(()),
        "[8.2.01-read] matching read target must be accepted"
    );
    assert_eq!(
        acceptance.accept_mutation_target(&mutation_target),
        Ok(()),
        "[8.2.01-mutation] matching mutation target must be accepted"
    );
}

#[test]
fn case_8_2_02_acceptance_reports_exact_read_identity_mismatch() {
    let (_fixture, first, second) = two_root_fixture();
    fs::write(first.join("managed.txt"), b"first").expect("write first managed fixture");
    fs::write(second.join("managed.txt"), b"second").expect("write second managed fixture");
    let Some(first_root) = read_root(&first) else {
        return;
    };
    let Some(second_root) = read_root(&second) else {
        return;
    };
    let path = RelativePath::parse("managed.txt").expect("parse managed path");
    let target = second_root
        .resolve_read(&path, super::super::ObjectClass::RegularFile)
        .expect("resolve read target in second root");
    let acceptance = first_root.acceptance(AuthorityAdapterKind::Source);
    let expected = PathError::AuthorityMismatch {
        owner: AuthorityAdapterKind::Source,
        root: first.display().to_string(),
        expected: first_root.identity(),
        actual: target.root_identity(),
    };

    assert_exact_error(
        "8.2.02-read-mismatch",
        acceptance
            .accept_read_target(&target)
            .expect_err("reject foreign read target"),
        expected,
    );
}

#[test]
fn case_8_2_03_acceptance_reports_exact_mutation_identity_mismatch() {
    let (_fixture, first, second) = two_root_fixture();
    fs::write(first.join("managed.txt"), b"first").expect("write first managed fixture");
    fs::write(second.join("managed.txt"), b"second").expect("write second managed fixture");
    let Some(first_root) = mutation_root(&first) else {
        return;
    };
    let Some(second_root) = mutation_root(&second) else {
        return;
    };
    let path = RelativePath::parse("managed.txt").expect("parse managed path");
    let target = second_root
        .resolve_mutation(&path, MutationIntent::Replace)
        .expect("resolve mutation target in second root");
    let acceptance = first_root.acceptance(AuthorityAdapterKind::Record);
    let expected = PathError::AuthorityMismatch {
        owner: AuthorityAdapterKind::Record,
        root: first.display().to_string(),
        expected: first_root.identity(),
        actual: target.root_identity(),
    };

    assert_exact_error(
        "8.2.03-mutation-mismatch",
        acceptance
            .accept_mutation_target(&target)
            .expect_err("reject foreign mutation target"),
        expected,
    );
}

#[test]
fn case_8_2_04_default_adapter_forwards_read_acceptance() {
    let (_fixture, first, second) = two_root_fixture();
    fs::write(first.join("managed.txt"), b"first").expect("write first managed fixture");
    fs::write(second.join("managed.txt"), b"second").expect("write second managed fixture");
    let Some(first_root) = read_root(&first) else {
        return;
    };
    let Some(second_root) = read_root(&second) else {
        return;
    };
    let path = RelativePath::parse("managed.txt").expect("parse managed path");
    let first_target = first_root
        .resolve_read(&path, super::super::ObjectClass::RegularFile)
        .expect("resolve first read target");
    let second_target = second_root
        .resolve_read(&path, super::super::ObjectClass::RegularFile)
        .expect("resolve second read target");
    let adapter = ForwardingAdapter {
        acceptance: first_root.acceptance(AuthorityAdapterKind::Process),
    };

    assert_eq!(
        adapter.accept_read_target(&first_target),
        Ok(()),
        "[8.2.04-read-match] default adapter read forwarding must accept"
    );
    let expected = PathError::AuthorityMismatch {
        owner: AuthorityAdapterKind::Process,
        root: first.display().to_string(),
        expected: first_root.identity(),
        actual: second_target.root_identity(),
    };
    assert_exact_error(
        "8.2.04-read-mismatch",
        adapter
            .accept_read_target(&second_target)
            .expect_err("default adapter read forwarding must reject"),
        expected,
    );
}

#[test]
fn case_8_2_05_default_adapter_forwards_mutation_acceptance() {
    let (_fixture, first, second) = two_root_fixture();
    fs::write(first.join("managed.txt"), b"first").expect("write first managed fixture");
    fs::write(second.join("managed.txt"), b"second").expect("write second managed fixture");
    let Some(first_root) = mutation_root(&first) else {
        return;
    };
    let Some(second_root) = mutation_root(&second) else {
        return;
    };
    let path = RelativePath::parse("managed.txt").expect("parse managed path");
    let first_target = first_root
        .resolve_mutation(&path, MutationIntent::Replace)
        .expect("resolve first mutation target");
    let second_target = second_root
        .resolve_mutation(&path, MutationIntent::Replace)
        .expect("resolve second mutation target");
    let adapter = ForwardingAdapter {
        acceptance: first_root.acceptance(AuthorityAdapterKind::Agent),
    };

    assert_eq!(
        adapter.accept_mutation_target(&first_target),
        Ok(()),
        "[8.2.05-mutation-match] default adapter mutation forwarding must accept"
    );
    let expected = PathError::AuthorityMismatch {
        owner: AuthorityAdapterKind::Agent,
        root: first.display().to_string(),
        expected: first_root.identity(),
        actual: second_target.root_identity(),
    };
    assert_exact_error(
        "8.2.05-mutation-mismatch",
        adapter
            .accept_mutation_target(&second_target)
            .expect_err("default adapter mutation forwarding must reject"),
        expected,
    );
}

#[test]
fn case_8_2_06_read_target_accessors_expose_parent_identity_and_clone() {
    let fixture = test_directory();
    let file_path = fixture.path().join("managed.txt");
    fs::write(&file_path, b"readable payload").expect("write read target fixture");
    let Some(root) = read_root(fixture.path()) else {
        return;
    };
    let relative = RelativePath::parse("managed.txt").expect("parse read target path");
    let target = root
        .resolve_read(&relative, super::super::ObjectClass::RegularFile)
        .expect("resolve read target");

    assert_eq!(
        target.relative_path(),
        &relative,
        "[8.2.06-relative] read target must expose its exact relative path"
    );
    assert_eq!(
        target.root_identity(),
        root.identity(),
        "[8.2.06-root-identity] read target must expose its owning root identity"
    );
    assert_eq!(
        target
            .parent_identity()
            .expect("read target parent identity"),
        root.identity(),
        "[8.2.06-parent] direct read target parent must be the authority root"
    );
    let mut clone = target.try_clone_file().expect("clone read target handle");
    let mut contents = String::new();
    clone
        .read_to_string(&mut contents)
        .expect("read cloned target handle");
    assert_eq!(
        contents, "readable payload",
        "[8.2.06-clone] cloned read handle must retain the target contents"
    );
    assert_eq!(
        target.identity(),
        root.resolve_read(&relative, super::super::ObjectClass::RegularFile)
            .expect("resolve target identity for comparison")
            .identity(),
        "[8.2.06-identity] read target identity must remain stable across accessors"
    );
}

#[test]
fn case_8_5_01_closed_read_handle_reports_exact_clone_error() {
    // Run the descriptor-close witness in a process that has no sibling tests
    // able to reuse the closed descriptor between close(2) and dup(2).
    if run_in_isolated_child(
        "platform::authority::tests::coverage_tests::adapters::case_8_5_01_closed_read_handle_reports_exact_clone_error",
    ) {
        return;
    }
    let fixture = test_directory();
    let file_path = fixture.path().join("closed-handle.txt");
    fs::write(&file_path, b"readable payload").expect("write closed-handle fixture");
    let Some(root) = read_root(fixture.path()) else {
        return;
    };
    let relative = RelativePath::parse("closed-handle.txt").expect("parse closed-handle path");
    let mut target = ManuallyDrop::new(
        root.resolve_read(&relative, super::super::ObjectClass::RegularFile)
            .expect("resolve read target"),
    );

    // Keep a replacement descriptor open before closing the target descriptor.
    // This prevents the old invalid File from closing an unrelated descriptor
    // if the operating system reuses its number.
    let replacement = fs::File::open(fixture.path()).expect("reopen fixture directory");
    let raw_fd = target.handle.as_raw_fd();
    let closed_fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
    drop(closed_fd);
    let error = target
        .try_clone_file()
        .expect_err("a closed read descriptor must not clone");
    assert!(matches!(
        error,
        PathError::Io {
            operation,
            path,
            code: Some(9),
            kind,
        } if operation == "clone read handle" && path == "closed-handle.txt" && !kind.is_empty()
    ));

    // Replace the closed descriptor before dropping the target so only the
    // deliberately closed descriptor is invalidated by this test.
    let old_handle = std::mem::replace(&mut target.handle, replacement);
    // The old File owns a descriptor that this test deliberately closed.
    // Forget it instead of triggering Rust's IO-safety abort on drop.
    std::mem::forget(old_handle);
    drop(ManuallyDrop::into_inner(target));
}

#[test]
fn case_8_2_07_existing_mutation_target_accessors_and_revalidation_are_stable() {
    let fixture = test_directory();
    fs::write(fixture.path().join("managed.txt"), b"mutable payload")
        .expect("write mutation target fixture");
    let Some(root) = mutation_root(fixture.path()) else {
        return;
    };
    let relative = RelativePath::parse("managed.txt").expect("parse existing mutation path");
    let target = root
        .resolve_mutation(&relative, MutationIntent::Replace)
        .expect("resolve existing mutation target");
    let expected_identity = target.identity().expect("existing target has identity");

    assert_eq!(
        target.root_identity(),
        root.identity(),
        "[8.2.07-root-identity] mutation target must expose its owning root identity"
    );
    assert_eq!(
        target.relative_path(),
        &relative,
        "[8.2.07-relative] mutation target must expose its exact relative path"
    );
    assert_eq!(
        target.intent(),
        MutationIntent::Replace,
        "[8.2.07-intent] existing target must retain the requested mutation intent"
    );
    assert_eq!(
        target.identity(),
        Some(expected_identity),
        "[8.2.07-identity] existing target must expose its object identity"
    );
    assert_eq!(
        target.revalidate(),
        Ok(()),
        "[8.2.07-revalidate] existing target must revalidate in place"
    );
    let file = target.into_file().expect("existing target becomes a file");
    assert_eq!(
        file.metadata()
            .expect("inspect existing mutation handle")
            .len(),
        b"mutable payload".len() as u64,
        "[8.2.07-into-file] existing target file must retain its content"
    );
}

#[test]
fn case_8_2_08_create_candidate_revalidates_and_creates_inside_root() {
    let fixture = test_directory();
    let Some(root) = mutation_root(fixture.path()) else {
        return;
    };
    let relative = RelativePath::parse("new-managed.txt").expect("parse create candidate path");
    let target = root
        .resolve_mutation(&relative, MutationIntent::CreateExclusive)
        .expect("resolve absent create candidate");

    assert_eq!(
        target.identity(),
        None,
        "[8.2.08-identity] absent create candidate must have no object identity"
    );
    assert_eq!(
        target.root_identity(),
        root.identity(),
        "[8.2.08-root-identity] candidate must expose its owning root identity"
    );
    assert_eq!(
        target.relative_path(),
        &relative,
        "[8.2.08-relative] candidate must expose its exact relative path"
    );
    assert_eq!(
        target.intent(),
        MutationIntent::CreateExclusive,
        "[8.2.08-intent] candidate must retain CreateExclusive intent"
    );
    assert_eq!(
        target.revalidate(),
        Ok(()),
        "[8.2.08-revalidate] absent candidate must revalidate while absent"
    );
    let mut file = target
        .create_exclusive()
        .expect("create candidate inside authority root");
    use std::io::Write;
    file.write_all(b"created payload")
        .expect("write through created target handle");
    assert_eq!(
        fs::read(fixture.path().join("new-managed.txt")).expect("read created target"),
        b"created payload",
        "[8.2.08-created] create candidate must materialize at the declared relative path"
    );
}

#[test]
fn case_8_2_09_candidate_into_file_reports_exact_not_found_error() {
    let fixture = test_directory();
    let Some(root) = mutation_root(fixture.path()) else {
        return;
    };
    let relative = RelativePath::parse("not-yet-created.txt").expect("parse absent path");
    let target = root
        .resolve_mutation(&relative, MutationIntent::CreateExclusive)
        .expect("resolve absent create candidate");

    assert_exact_error(
        "8.2.09-candidate-into-file",
        target
            .into_file()
            .expect_err("absent candidate has no file handle"),
        PathError::NotFound {
            path: "not-yet-created.txt".to_owned(),
        },
    );
}

#[test]
fn case_8_2_10_wrong_create_intent_returns_exact_typed_error() {
    let fixture = test_directory();
    fs::write(fixture.path().join("existing.txt"), b"existing")
        .expect("write existing intent fixture");
    let Some(root) = mutation_root(fixture.path()) else {
        return;
    };
    let target = root
        .resolve_mutation(
            &RelativePath::parse("existing.txt").expect("parse existing intent path"),
            MutationIntent::Replace,
        )
        .expect("resolve existing replacement target");

    assert_exact_error(
        "8.2.10-wrong-intent",
        target
            .create_exclusive()
            .expect_err("Replace target must not be used for exclusive creation"),
        PathError::Io {
            operation: "create exclusive target".to_owned(),
            path: "existing.txt".to_owned(),
            kind: "mutation intent is not CreateExclusive".to_owned(),
            code: None,
        },
    );
}

#[test]
fn case_8_2_11_registry_registers_root_and_read_target_and_rejects_duplicates() {
    let fixture = test_directory();
    fs::write(fixture.path().join("read.txt"), b"read").expect("write registry read fixture");
    let Some(root) = read_root(fixture.path()) else {
        return;
    };
    let relative = RelativePath::parse("read.txt").expect("parse registry read path");
    let target = root
        .resolve_read(&relative, super::super::ObjectClass::RegularFile)
        .expect("resolve registry read target");
    let mut registry = AuthorityRegistry::default();

    assert_eq!(
        registry.register_root(&root, "root"),
        Ok(()),
        "[8.2.11-root-register] registry must register an authority root"
    );
    assert!(
        registry.contains(root.identity()),
        "[8.2.11-root-contains] registry must contain the registered root identity"
    );
    assert!(
        !registry.contains(target.identity()),
        "[8.2.11-read-absent] read identity must be absent before registration"
    );
    assert_eq!(
        registry.register_read_target(&target, "read"),
        Ok(()),
        "[8.2.11-read-register] registry must register a read target"
    );
    assert!(
        registry.contains(target.identity()),
        "[8.2.11-read-contains] registry must contain the registered read identity"
    );
    assert_exact_error(
        "8.2.11-root-duplicate",
        registry
            .register_root(&root, "root-again")
            .expect_err("duplicate root identity must fail"),
        PathError::DuplicateAuthority {
            label: "root-again".to_owned(),
            existing: "root".to_owned(),
            identity: root.identity(),
        },
    );
    assert_exact_error(
        "8.2.11-read-duplicate",
        registry
            .register_read_target(&target, "read-again")
            .expect_err("duplicate read identity must fail"),
        PathError::DuplicateAuthority {
            label: "read-again".to_owned(),
            existing: "read".to_owned(),
            identity: target.identity(),
        },
    );
}

#[test]
fn case_8_2_12_registry_registers_existing_mutation_and_rejects_duplicates() {
    let fixture = test_directory();
    fs::write(fixture.path().join("mutate.txt"), b"mutate")
        .expect("write registry mutation fixture");
    let Some(root) = mutation_root(fixture.path()) else {
        return;
    };
    let target = root
        .resolve_mutation(
            &RelativePath::parse("mutate.txt").expect("parse registry mutation path"),
            MutationIntent::Append,
        )
        .expect("resolve registry mutation target");
    let identity = target.identity().expect("existing mutation has identity");
    let mut registry = AuthorityRegistry::default();

    assert_eq!(
        registry.register_mutation_target(&target, "mutation"),
        Ok(()),
        "[8.2.12-register] registry must register an existing mutation target"
    );
    assert!(
        registry.contains(identity),
        "[8.2.12-contains] registry must contain the mutation identity"
    );
    assert_exact_error(
        "8.2.12-duplicate",
        registry
            .register_mutation_target(&target, "mutation-again")
            .expect_err("duplicate mutation identity must fail"),
        PathError::DuplicateAuthority {
            label: "mutation-again".to_owned(),
            existing: "mutation".to_owned(),
            identity,
        },
    );
}

#[test]
fn case_8_2_13_registry_rejects_absent_candidate_overlap_and_contains_false() {
    let fixture = test_directory();
    let Some(root) = mutation_root(fixture.path()) else {
        return;
    };
    let relative = RelativePath::parse("candidate.txt").expect("parse registry candidate path");
    let target = root
        .resolve_mutation(&relative, MutationIntent::CreateExclusive)
        .expect("resolve registry create candidate");
    let mut registry = AuthorityRegistry::default();

    assert_exact_error(
        "8.2.13-overlap",
        registry
            .register_mutation_target(&target, "candidate")
            .expect_err("absent candidate must not register as an authority"),
        PathError::AuthorityOverlap {
            path: "candidate.txt".to_owned(),
        },
    );
    assert!(
        !registry.contains(root.identity()),
        "[8.2.13-contains-false] rejected candidate must not add any identity"
    );
}

#[test]
fn case_8_2_14_free_typed_root_wrappers_preserve_root_accessors() {
    let fixture = test_directory();
    let read = match open_read_root::<MachineConfigRoot>(fixture.path()) {
        Ok(root) => root,
        Err(PathError::UnsupportedFilesystem { .. }) => return,
        Err(error) => panic!("supported free read-root fixture failed: {error}"),
    };
    let mutation = match open_mutation_root::<DestinationRepositoryRoot>(fixture.path()) {
        Ok(root) => root,
        Err(PathError::UnsupportedFilesystem { .. }) => return,
        Err(error) => panic!("supported free mutation-root fixture failed: {error}"),
    };

    assert_eq!(
        read.display_path().as_path(),
        fixture.path(),
        "[8.2.14-read-path] free read-root wrapper must preserve the root path"
    );
    assert_eq!(
        mutation.display_path().as_path(),
        fixture.path(),
        "[8.2.14-mutation-path] free mutation-root wrapper must preserve the root path"
    );
    assert_eq!(
        read.identity(),
        mutation.identity(),
        "[8.2.14-identity] free typed wrappers must observe one root identity"
    );
}

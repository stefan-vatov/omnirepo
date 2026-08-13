#![allow(dead_code, unused_imports, clippy::duplicate_mod, unreachable_patterns)]

pub(crate) use super::*;
use super::{
    AbsolutePath, AuthorityRegistry, AuthorityRoot, DestinationRepositoryRoot, FilesystemKind,
    Mutate, MutationIntent, ObjectClass, PathError, ReadOnly, RelativePath,
};
use std::{
    ffi::{CString, OsString},
    fs,
    io::{Read, Write},
    mem::ManuallyDrop,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::Path,
    process::Command,
    sync::{Arc, Barrier},
    thread,
};
use tempfile::{Builder, TempDir};

use self::coverage_tests::capability;

const CLOSED_DESCRIPTOR_CHILD: &str = "OMNIREPO_AUTHORITY_CLOSED_DESCRIPTOR_CHILD";

mod coverage_tests;

fn test_directory() -> TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("create test base");
    let fixture = Builder::new()
        .prefix("authority-confinement-")
        .tempdir_in(base)
        .expect("create authority fixture");
    capability::report(fixture.path());
    fixture
}

fn read_root(path: &Path) -> Option<AuthorityRoot<DestinationRepositoryRoot, ReadOnly>> {
    match AuthorityRoot::<DestinationRepositoryRoot, ReadOnly>::open(path) {
        Ok(root) => Some(root),
        Err(PathError::UnsupportedFilesystem { .. }) => None,
        Err(error) => panic!("supported fixture root failed: {error}"),
    }
}

fn mutation_root(path: &Path) -> Option<AuthorityRoot<DestinationRepositoryRoot, Mutate>> {
    match AuthorityRoot::<DestinationRepositoryRoot, Mutate>::open(path) {
        Ok(root) => Some(root),
        Err(PathError::UnsupportedFilesystem { .. }) => None,
        Err(error) => panic!("supported fixture root failed: {error}"),
    }
}

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
        .expect("run isolated closed-descriptor authority test");
    let summary = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && summary.contains("1 passed"),
        "isolated closed-descriptor test must run exactly one test: {test_name}\n{summary}"
    );
    true
}

#[test]
#[should_panic(expected = "must run exactly one test")]
fn isolated_child_helper_fails_on_wrong_selection() {
    run_in_isolated_child("platform::authority::tests::no_such_closed_descriptor_test");
}

const REQUIRED_CONTAINMENT_ROWS: &[(&str, &str)] = &[
    ("traversal", "all-supported"),
    ("case-unicode-equivalence", "filesystem-capability"),
    ("root-alias", "all-supported"),
    ("leaf-alias", "symlink"),
    ("mount-identity", "linux-local-filesystem"),
    ("hard-link", "hard-link"),
    ("special-object", "fifo"),
    ("concurrent-swap", "symlink"),
    ("nested-fleet-roots", "all-supported"),
    ("unsupported-filesystem", "linux-proc"),
    ("independent-peer-outcome", "all-supported"),
];

const DECLARED_CONTAINMENT_ROWS: &[(&str, &str)] = &[
    ("traversal", "all-supported"),
    ("case-unicode-equivalence", "filesystem-capability"),
    ("root-alias", "all-supported"),
    ("leaf-alias", "symlink"),
    ("mount-identity", "linux-local-filesystem"),
    ("hard-link", "hard-link"),
    ("special-object", "fifo"),
    ("concurrent-swap", "symlink"),
    ("nested-fleet-roots", "all-supported"),
    ("unsupported-filesystem", "linux-proc"),
    ("independent-peer-outcome", "all-supported"),
];

#[test]
fn containment_matrix_declares_every_required_boundary() {
    for required in REQUIRED_CONTAINMENT_ROWS {
        assert!(
            DECLARED_CONTAINMENT_ROWS.contains(required),
            "containment matrix is missing row {:?} with capability gate {:?}",
            required.0,
            required.1
        );
    }
}

#[test]
fn relative_and_absolute_paths_reject_ambiguous_forms() {
    let normalized = RelativePath::parse("one//./two").expect("valid normalized path");
    let direct = RelativePath::parse("one/two").expect("valid direct path");
    assert_eq!(normalized, direct);
    assert_eq!(
        RelativePath::parse(r"one\two")
            .unwrap()
            .components()
            .count(),
        1
    );

    for invalid in ["", ".", "//", "/absolute", "../escape", "one/../escape"] {
        assert!(
            matches!(
                RelativePath::parse(invalid),
                Err(PathError::InvalidRelativePath { .. })
            ),
            "path should be rejected: {invalid:?}"
        );
    }
    assert!(matches!(
        RelativePath::parse("nul\0component"),
        Err(PathError::InvalidRelativePath { .. })
    ));

    assert!(matches!(
        AbsolutePath::parse("relative/root"),
        Err(PathError::InvalidAbsolutePath { .. })
    ));
    assert!(matches!(
        AbsolutePath::parse("/authority/../escape"),
        Err(PathError::InvalidAbsolutePath { .. })
    ));
    assert!(matches!(
        AbsolutePath::parse("/nul\0root"),
        Err(PathError::InvalidAbsolutePath { .. })
    ));
}

#[test]
fn absolute_paths_reject_non_utf8_components_before_authority_use() {
    let raw_path = OsString::from_vec(b"/tmp/omnirepo-\xff".to_vec());
    let path = Path::new(&raw_path);
    assert!(matches!(
        AbsolutePath::from_path(path),
        Err(PathError::InvalidAbsolutePath { path, reason })
            if path.contains("omnirepo") && reason == "authority paths must be UTF-8"
    ));
}

#[test]
fn case_and_unicode_spellings_use_filesystem_identity_without_normalization() {
    let fixture = test_directory();
    let Some(root) = read_root(fixture.path()) else {
        return;
    };

    for (first_name, second_name) in [("CaseSensitive", "casesensitive"), ("café", "café")] {
        let first = RelativePath::parse(first_name).expect("first spelling is valid UTF-8");
        let second = RelativePath::parse(second_name).expect("second spelling is valid UTF-8");
        assert_ne!(first, second, "the input spellings remain exact text");
        fs::write(fixture.path().join(first_name), b"first\n").expect("write first spelling");
        fs::write(fixture.path().join(second_name), b"second\n").expect("write second spelling");

        let first_target = root
            .resolve_read(&first, ObjectClass::RegularFile)
            .expect("first spelling resolves inside the root");
        let second_target = root
            .resolve_read(&second, ObjectClass::RegularFile)
            .expect("second spelling resolves inside the root");
        let mut registry = AuthorityRegistry::default();
        registry
            .register_read_target(&first_target, first_name)
            .expect("first spelling registers");
        if first_target.identity() == second_target.identity() {
            assert!(matches!(
                registry.register_read_target(&second_target, second_name),
                Err(PathError::DuplicateAuthority { .. })
            ));
        } else {
            registry
                .register_read_target(&second_target, second_name)
                .expect("distinct filesystem objects remain distinct authorities");
        }
    }
}

#[test]
fn root_open_rejects_symlinked_roots_and_non_directories() {
    let fixture = test_directory();
    let real = fixture.path().join("real");
    fs::create_dir(&real).expect("create real root");
    let link = fixture.path().join("root-link");
    std::os::unix::fs::symlink(&real, &link).expect("create root symlink");
    assert!(matches!(
        AuthorityRoot::<DestinationRepositoryRoot, ReadOnly>::open(&link),
        Err(PathError::LinkLikeObject { .. })
    ));

    let file = fixture.path().join("root-file");
    fs::write(&file, b"not a directory").expect("create root file");
    assert!(matches!(
        AuthorityRoot::<DestinationRepositoryRoot, ReadOnly>::open(&file),
        Err(PathError::InvalidAuthorityRoot { .. })
    ));
}

#[test]
fn intermediate_and_leaf_links_fail_without_following() {
    let fixture = test_directory();
    let Some(root) = read_root(fixture.path()) else {
        return;
    };
    let real_directory = fixture.path().join("real-directory");
    fs::create_dir(&real_directory).expect("create real directory");
    fs::write(real_directory.join("payload"), b"authoritative").expect("write payload");

    let intermediate_link = fixture.path().join("intermediate-link");
    std::os::unix::fs::symlink(&real_directory, &intermediate_link)
        .expect("create intermediate symlink");
    let intermediate_path = RelativePath::parse("intermediate-link/payload").unwrap();
    assert!(matches!(
        root.resolve_read(&intermediate_path, ObjectClass::RegularFile),
        Err(PathError::LinkLikeObject { .. })
    ));

    let leaf_link = fixture.path().join("leaf-link");
    std::os::unix::fs::symlink(real_directory.join("payload"), &leaf_link)
        .expect("create leaf symlink");
    let leaf_path = RelativePath::parse("leaf-link").unwrap();
    assert!(matches!(
        root.resolve_read(&leaf_path, ObjectClass::RegularFile),
        Err(PathError::LinkLikeObject { .. })
    ));

    let payload_path = RelativePath::parse("real-directory/payload").unwrap();
    let target = root
        .resolve_read(&payload_path, ObjectClass::RegularFile)
        .expect("regular target is readable");
    let mut contents = String::new();
    target
        .try_clone_file()
        .expect("clone read handle")
        .read_to_string(&mut contents)
        .expect("read through checked handle");
    assert_eq!(contents, "authoritative");
}

#[test]
fn root_relative_read_duplicates_the_authority_root_exactly() {
    let fixture = test_directory();
    let Some(root) = read_root(fixture.path()) else {
        return;
    };
    let target = root
        .resolve_read(&RelativePath::root(), ObjectClass::Directory)
        .expect("the explicit root path resolves as a directory");

    assert_eq!(target.relative_path(), &RelativePath::root());
    assert_eq!(target.identity(), root.identity());
    assert_eq!(target.root_identity(), root.identity());
    assert_eq!(
        target.parent_identity().expect("root parent identity"),
        root.identity()
    );
    target
        .try_clone_file()
        .expect("the duplicated root handle remains cloneable");
}

#[test]
fn non_directory_intermediate_objects_fail_closed() {
    let fixture = test_directory();
    let Some(root) = read_root(fixture.path()) else {
        return;
    };
    fs::write(fixture.path().join("file"), b"payload").expect("write file");
    let path = RelativePath::parse("file/child").unwrap();
    assert!(matches!(
        root.resolve_read(&path, ObjectClass::RegularFile),
        Err(PathError::UnsupportedObject {
            expected: ObjectClass::Directory,
            ..
        })
    ));
}

#[test]
fn special_files_are_rejected_as_non_regular_objects() {
    let fixture = test_directory();
    let Some(root) = read_root(fixture.path()) else {
        return;
    };
    let fifo = fixture.path().join("fifo");
    let fifo_c = CString::new(fifo.as_os_str().as_bytes()).expect("fifo path has no NUL");
    let result = unsafe { mkfifo(fifo_c.as_ptr(), 0o600) };
    assert_eq!(result, 0, "create fifo fixture");
    let path = RelativePath::parse("fifo").unwrap();
    assert!(matches!(
        root.resolve_read(&path, ObjectClass::RegularFile),
        Err(PathError::UnsupportedObject {
            expected: ObjectClass::RegularFile,
            ..
        })
    ));
}

#[test]
fn hard_link_aliases_collide_and_are_not_mutable() {
    let fixture = test_directory();
    let Some(read_root) = read_root(fixture.path()) else {
        return;
    };
    let Some(mutation_root) = mutation_root(fixture.path()) else {
        return;
    };
    fs::write(fixture.path().join("original"), b"payload").expect("write original");
    fs::hard_link(
        fixture.path().join("original"),
        fixture.path().join("alias"),
    )
    .expect("create hard-link alias");

    let original = read_root
        .resolve_read(
            &RelativePath::parse("original").unwrap(),
            ObjectClass::RegularFile,
        )
        .expect("read original");
    let alias = read_root
        .resolve_read(
            &RelativePath::parse("alias").unwrap(),
            ObjectClass::RegularFile,
        )
        .expect("read alias");
    assert_eq!(original.identity(), alias.identity());

    let mut registry = AuthorityRegistry::default();
    registry
        .register_read_target(&original, "original")
        .expect("register first identity");
    assert!(matches!(
        registry.register_read_target(&alias, "alias"),
        Err(PathError::DuplicateAuthority { .. })
    ));

    assert!(matches!(
        mutation_root.resolve_mutation(
            &RelativePath::parse("alias").unwrap(),
            MutationIntent::Replace,
        ),
        Err(PathError::UnsafeHardLink { links: 2, .. })
    ));
}

#[test]
fn hard_link_growth_after_mutation_resolution_fails_closed() {
    let fixture = test_directory();
    let Some(root) = mutation_root(fixture.path()) else {
        return;
    };
    let path = RelativePath::parse("managed").expect("parse managed path");
    let managed = fixture.path().join("managed");
    fs::write(&managed, b"authoritative").expect("write managed file");
    let target = root
        .resolve_mutation(&path, MutationIntent::Replace)
        .expect("resolve a single-link target");

    fs::hard_link(&managed, fixture.path().join("managed-alias"))
        .expect("add a second hard-link name after resolution");
    assert!(matches!(
        target.revalidate(),
        Err(PathError::UnsafeHardLink { path, links: 2 }) if path == "managed"
    ));
    assert_eq!(
        fs::read(&managed).expect("read managed file"),
        b"authoritative"
    );
}

#[test]
fn create_candidate_appearance_fails_closed_without_mutation() {
    let fixture = test_directory();
    let Some(root) = mutation_root(fixture.path()) else {
        return;
    };
    let path = RelativePath::parse("appeared").expect("parse candidate path");
    let target = root
        .resolve_mutation(&path, MutationIntent::CreateExclusive)
        .expect("resolve absent create candidate");
    fs::write(fixture.path().join("appeared"), b"peer-created")
        .expect("make the candidate appear before revalidation");

    assert!(matches!(
        target.revalidate(),
        Err(PathError::ConcurrentReplacement { path, reason })
            if path == "appeared" && reason.contains("authority leaf appeared")
    ));
    assert_eq!(
        fs::read(fixture.path().join("appeared")).expect("read appeared candidate"),
        b"peer-created"
    );
}

#[test]
fn create_exclusive_is_root_relative_and_rechecked() {
    let fixture = test_directory();
    let Some(root) = mutation_root(fixture.path()) else {
        return;
    };
    let path = RelativePath::parse("new-file").unwrap();
    let target = root
        .resolve_mutation(&path, MutationIntent::CreateExclusive)
        .expect("missing target is a contained create candidate");
    assert!(target.identity().is_none());
    let mut file = target.create_exclusive().expect("create contained file");
    use std::io::Write;
    file.write_all(b"created").expect("write created file");
    drop(file);

    let existing = root
        .resolve_mutation(&path, MutationIntent::CreateExclusive)
        .expect("existing target is resolved before create is attempted");
    assert!(matches!(
        existing.create_exclusive(),
        Err(PathError::Io { code: Some(17), .. }) | Err(PathError::UnsafeHardLink { .. })
    ));
}

#[test]
fn mutation_parent_clone_reports_exact_closed_descriptor_error() {
    if run_in_isolated_child(
        "platform::authority::tests::mutation_parent_clone_reports_exact_closed_descriptor_error",
    ) {
        return;
    }
    let fixture = test_directory();
    let managed = fixture.path().join("managed");
    fs::write(&managed, b"authoritative").expect("write managed target");
    let Some(root) = mutation_root(fixture.path()) else {
        return;
    };
    let relative = RelativePath::parse("managed").expect("parse managed target");
    let mut target = ManuallyDrop::new(
        root.resolve_mutation(&relative, MutationIntent::Replace)
            .expect("resolve managed target"),
    );

    let replacement = fs::File::open(fixture.path()).expect("open replacement parent handle");
    let raw_fd = target.parent.as_raw_fd();
    drop(unsafe { OwnedFd::from_raw_fd(raw_fd) });
    let error = target
        .clone_parent()
        .expect_err("closed mutation parent must not clone");
    assert!(matches!(
        error,
        PathError::Io {
            operation,
            path,
            code: Some(9),
            kind,
        } if operation == "clone mutation parent" && path == "managed" && !kind.is_empty()
    ));

    let old_parent = std::mem::replace(&mut target.parent, replacement);
    std::mem::forget(old_parent);
    drop(ManuallyDrop::into_inner(target));
}

#[test]
fn closed_sync_descriptors_report_exact_file_and_directory_errors() {
    if run_in_isolated_child(
        "platform::authority::tests::closed_sync_descriptors_report_exact_file_and_directory_errors",
    ) {
        return;
    }
    let fixture = test_directory();

    let file_path = fixture.path().join("sync-file");
    fs::write(&file_path, b"payload").expect("write sync file");
    let mut file = ManuallyDrop::new(fs::File::open(&file_path).expect("open sync file"));
    let replacement_file = fs::File::open(fixture.path()).expect("open replacement file handle");
    let raw_file_fd = file.as_raw_fd();
    drop(unsafe { OwnedFd::from_raw_fd(raw_file_fd) });
    let file_error =
        super::sync_file(&file, "sync-file").expect_err("closed mutation file must not sync");
    assert!(matches!(
        file_error,
        PathError::Io {
            operation,
            path,
            code: Some(9),
            kind,
        } if operation == "sync mutation file" && path == "sync-file" && !kind.is_empty()
    ));
    let old_file = std::mem::replace(&mut *file, replacement_file);
    std::mem::forget(old_file);
    unsafe { ManuallyDrop::drop(&mut file) };

    let mut directory =
        ManuallyDrop::new(fs::File::open(fixture.path()).expect("open sync directory"));
    let replacement_directory =
        fs::File::open(fixture.path()).expect("open replacement directory handle");
    let raw_directory_fd = directory.as_raw_fd();
    drop(unsafe { OwnedFd::from_raw_fd(raw_directory_fd) });
    let directory_error = super::sync_directory(&directory, "sync-directory")
        .expect_err("closed mutation directory must not sync");
    assert!(matches!(
        directory_error,
        PathError::Io {
            operation,
            path,
            code: Some(9),
            kind,
        } if operation == "sync mutation directory" && path == "sync-directory" && !kind.is_empty()
    ));
    let old_directory = std::mem::replace(&mut *directory, replacement_directory);
    std::mem::forget(old_directory);
    unsafe { ManuallyDrop::drop(&mut directory) };
}

#[test]
fn leaf_rename_and_symlink_swap_is_rejected_before_use() {
    let fixture = test_directory();
    let authority = fixture.path().join("authority");
    let outside = fixture.path().join("outside");
    fs::create_dir(&authority).expect("create authority root");
    fs::create_dir(&outside).expect("create outside directory");
    let authority_target = authority.join("target");
    let outside_target = outside.join("target");
    fs::write(&authority_target, b"authoritative").expect("write authority target");

    let Some(root) = mutation_root(&authority) else {
        return;
    };
    let target = root
        .resolve_mutation(
            &RelativePath::parse("target").expect("parse target"),
            MutationIntent::Replace,
        )
        .expect("resolve checked mutation target");

    let barrier = Arc::new(Barrier::new(2));
    let attacker_barrier = Arc::clone(&barrier);
    let attacker_authority_target = authority_target.clone();
    let attacker_outside_target = outside_target.clone();
    let attacker = thread::spawn(move || {
        attacker_barrier.wait();
        fs::rename(&attacker_authority_target, &attacker_outside_target)
            .expect("move the originally authorized object out of the root");
        std::os::unix::fs::symlink(&attacker_outside_target, &attacker_authority_target)
            .expect("replace the authority name with an outside symlink");
        attacker_barrier.wait();
    });

    barrier.wait();
    barrier.wait();
    attacker.join().expect("swap thread completes");

    match target.into_file() {
        Err(PathError::ConcurrentReplacement { reason, .. }) => {
            assert!(
                reason.contains("authority leaf"),
                "replacement error must identify the leaf boundary: {reason}"
            );
        }
        Ok(file) => {
            file.set_len(0)
                .expect("the pre-revalidation API can truncate its returned handle");
            assert_eq!(
                fs::read(&outside_target).expect("read moved target"),
                b"authoritative",
                "RED: resolve then use mutates the object after it leaves the authority root"
            );
        }
        Err(error) => panic!("unexpected replacement result: {error}"),
    }
}

#[test]
fn intermediate_ancestor_rename_and_symlink_swap_is_rejected_before_use() {
    let fixture = test_directory();
    let authority = fixture.path().join("authority");
    let outside = fixture.path().join("outside");
    let ancestor = authority.join("ancestor");
    let moved_ancestor = outside.join("ancestor-moved");
    let authority_target = ancestor.join("target");
    let outside_target = moved_ancestor.join("target");
    fs::create_dir(&authority).expect("create authority root");
    fs::create_dir(&outside).expect("create outside directory");
    fs::create_dir(&ancestor).expect("create authority ancestor");
    fs::write(&authority_target, b"authoritative").expect("write authority target");

    let Some(root) = mutation_root(&authority) else {
        return;
    };
    let target = root
        .resolve_mutation(
            &RelativePath::parse("ancestor/target").expect("parse target"),
            MutationIntent::Replace,
        )
        .expect("resolve checked mutation target");

    let barrier = Arc::new(Barrier::new(2));
    let attacker_barrier = Arc::clone(&barrier);
    let attacker_ancestor = ancestor.clone();
    let attacker_moved_ancestor = moved_ancestor.clone();
    let attacker = thread::spawn(move || {
        attacker_barrier.wait();
        fs::rename(&attacker_ancestor, &attacker_moved_ancestor)
            .expect("move the originally authorized ancestor out of the root");
        std::os::unix::fs::symlink(&attacker_moved_ancestor, &attacker_ancestor)
            .expect("replace the ancestor name with an outside symlink");
        attacker_barrier.wait();
    });

    barrier.wait();
    barrier.wait();
    attacker.join().expect("swap thread completes");

    match target.into_file() {
        Err(PathError::ConcurrentReplacement { .. }) => {}
        Ok(file) => {
            file.set_len(0)
                .expect("the pre-revalidation API can truncate its returned handle");
            assert_eq!(
                fs::read(&outside_target).expect("read moved target"),
                b"authoritative",
                "RED: an ancestor swap moves the resolved handle outside the authority root"
            );
        }
        Err(error) => panic!("unexpected replacement result: {error}"),
    }
}

#[test]
fn authority_root_rename_and_symlink_swap_is_rejected_before_use() {
    let fixture = test_directory();
    let authority = fixture.path().join("authority");
    let moved_authority = fixture.path().join("outside-root");
    let authority_target = authority.join("target");
    let outside_target = moved_authority.join("target");
    fs::create_dir(&authority).expect("create authority root");
    fs::write(&authority_target, b"authoritative").expect("write authority target");

    let Some(root) = mutation_root(&authority) else {
        return;
    };
    let target = root
        .resolve_mutation(
            &RelativePath::parse("target").expect("parse target"),
            MutationIntent::Replace,
        )
        .expect("resolve checked mutation target");

    let barrier = Arc::new(Barrier::new(2));
    let attacker_barrier = Arc::clone(&barrier);
    let attacker_authority = authority.clone();
    let attacker_moved_authority = moved_authority.clone();
    let attacker = thread::spawn(move || {
        attacker_barrier.wait();
        fs::rename(&attacker_authority, &attacker_moved_authority)
            .expect("move the originally authorized root out of its declared path");
        std::os::unix::fs::symlink(&attacker_moved_authority, &attacker_authority)
            .expect("replace the root name with an outside symlink");
        attacker_barrier.wait();
    });

    barrier.wait();
    barrier.wait();
    attacker.join().expect("swap thread completes");

    match target.into_file() {
        Err(PathError::ConcurrentReplacement { .. }) => {}
        Ok(file) => {
            file.set_len(0)
                .expect("the pre-revalidation API can truncate its returned handle");
            assert_eq!(
                fs::read(&outside_target).expect("read moved target"),
                b"authoritative",
                "RED: a root swap moves the resolved handle outside the authority root"
            );
        }
        Err(error) => panic!("unexpected replacement result: {error}"),
    }
}

#[test]
fn leaf_create_swap_fails_closed_without_external_creation() {
    let fixture = test_directory();
    let authority = fixture.path().join("authority");
    let outside = fixture.path().join("outside");
    let authority_target = authority.join("new-file");
    let outside_target = outside.join("new-file");
    fs::create_dir(&authority).expect("create authority root");
    fs::create_dir(&outside).expect("create outside directory");

    let Some(root) = mutation_root(&authority) else {
        return;
    };
    let target = root
        .resolve_mutation(
            &RelativePath::parse("new-file").expect("parse target"),
            MutationIntent::CreateExclusive,
        )
        .expect("resolve missing create candidate");

    let barrier = Arc::new(Barrier::new(2));
    let attacker_barrier = Arc::clone(&barrier);
    let attacker_authority_target = authority_target.clone();
    let attacker_outside_target = outside_target.clone();
    let attacker = thread::spawn(move || {
        attacker_barrier.wait();
        std::os::unix::fs::symlink(&attacker_outside_target, &attacker_authority_target)
            .expect("replace the create candidate with an outside symlink");
        attacker_barrier.wait();
    });

    barrier.wait();
    barrier.wait();
    attacker.join().expect("swap thread completes");

    assert!(matches!(
        target.create_exclusive(),
        Err(PathError::ConcurrentReplacement { .. })
    ));
    assert!(
        !outside_target.exists(),
        "the outside symlink target must not be created"
    );
}

#[test]
fn intermediate_create_swap_fails_closed_without_external_creation() {
    let fixture = test_directory();
    let authority = fixture.path().join("authority");
    let outside = fixture.path().join("outside");
    let ancestor = authority.join("ancestor");
    let moved_ancestor = outside.join("ancestor-moved");
    let outside_target = moved_ancestor.join("new-file");
    fs::create_dir(&authority).expect("create authority root");
    fs::create_dir(&outside).expect("create outside directory");
    fs::create_dir(&ancestor).expect("create authority ancestor");

    let Some(root) = mutation_root(&authority) else {
        return;
    };
    let target = root
        .resolve_mutation(
            &RelativePath::parse("ancestor/new-file").expect("parse target"),
            MutationIntent::CreateExclusive,
        )
        .expect("resolve missing create candidate");

    let barrier = Arc::new(Barrier::new(2));
    let attacker_barrier = Arc::clone(&barrier);
    let attacker_ancestor = ancestor.clone();
    let attacker_moved_ancestor = moved_ancestor.clone();
    let attacker = thread::spawn(move || {
        attacker_barrier.wait();
        fs::rename(&attacker_ancestor, &attacker_moved_ancestor)
            .expect("move the originally authorized ancestor out of the root");
        std::os::unix::fs::symlink(&attacker_moved_ancestor, &attacker_ancestor)
            .expect("replace the ancestor name with an outside symlink");
        attacker_barrier.wait();
    });

    barrier.wait();
    barrier.wait();
    attacker.join().expect("swap thread completes");

    assert!(matches!(
        target.create_exclusive(),
        Err(PathError::ConcurrentReplacement { .. })
    ));
    assert!(
        !outside_target.exists(),
        "an ancestor swap must not create a file in the moved directory"
    );
    assert!(authority.join("ancestor").is_symlink());
}

#[test]
fn authority_root_create_swap_fails_closed_without_external_creation() {
    let fixture = test_directory();
    let authority = fixture.path().join("authority");
    let moved_authority = fixture.path().join("outside-root");
    let outside_target = moved_authority.join("new-file");
    fs::create_dir(&authority).expect("create authority root");

    let Some(root) = mutation_root(&authority) else {
        return;
    };
    let target = root
        .resolve_mutation(
            &RelativePath::parse("new-file").expect("parse target"),
            MutationIntent::CreateExclusive,
        )
        .expect("resolve missing create candidate");

    let barrier = Arc::new(Barrier::new(2));
    let attacker_barrier = Arc::clone(&barrier);
    let attacker_authority = authority.clone();
    let attacker_moved_authority = moved_authority.clone();
    let attacker = thread::spawn(move || {
        attacker_barrier.wait();
        fs::rename(&attacker_authority, &attacker_moved_authority)
            .expect("move the originally authorized root out of its declared path");
        std::os::unix::fs::symlink(&attacker_moved_authority, &attacker_authority)
            .expect("replace the root name with an outside symlink");
        attacker_barrier.wait();
    });

    barrier.wait();
    barrier.wait();
    attacker.join().expect("swap thread completes");

    assert!(matches!(
        target.create_exclusive(),
        Err(PathError::ConcurrentReplacement { .. })
    ));
    assert!(
        !outside_target.exists(),
        "a root swap must not create a file in the moved root"
    );
}

#[test]
fn aliases_of_an_authority_root_share_identity() {
    let fixture = test_directory();
    let Some(first) = read_root(fixture.path()) else {
        return;
    };
    let alias_path = fixture.path().join(".");
    let second = read_root(&alias_path).expect("dot alias has the same supported root");
    assert_eq!(first.identity(), second.identity());

    let mut registry = AuthorityRegistry::default();
    registry
        .register_root(&first, "first")
        .expect("register first root");
    assert!(matches!(
        registry.register_root(&second, "dot-alias"),
        Err(PathError::DuplicateAuthority { .. })
    ));
}

#[test]
fn nested_fleet_roots_mutate_only_the_declared_nested_root() {
    let fixture = test_directory();
    let outer_path = fixture.path().join("fleet");
    let nested_path = outer_path.join("nested");
    let peer_path = outer_path.join("peer");
    fs::create_dir_all(&nested_path).expect("create nested fleet root");
    fs::create_dir(&peer_path).expect("create independent peer root");
    fs::write(nested_path.join("managed"), b"nested-before\n").expect("write nested target");
    fs::write(peer_path.join("managed"), b"peer-before\n").expect("write peer target");

    let Some(outer) = read_root(&outer_path) else {
        return;
    };
    let Some(nested) = mutation_root(&nested_path) else {
        return;
    };
    let peer_before = fs::read(peer_path.join("managed")).expect("read peer target");
    assert_ne!(outer.identity(), nested.identity());

    let mut registry = AuthorityRegistry::default();
    registry
        .register_root(&outer, "fleet")
        .expect("outer fleet root registers");
    registry
        .register_root(&nested, "nested-fleet")
        .expect("nested root is an independent declared authority");

    assert!(RelativePath::parse("../peer/managed").is_err());
    let target = nested
        .resolve_mutation(
            &RelativePath::parse("managed").expect("nested target path"),
            MutationIntent::Replace,
        )
        .expect("nested target resolves");
    let mut file = target.into_file().expect("nested target revalidates");
    file.set_len(0).expect("truncate nested target only");
    file.write_all(b"nested-after\n")
        .expect("write nested target only");

    assert_eq!(
        fs::read(nested_path.join("managed")).expect("read nested target"),
        b"nested-after\n"
    );
    assert_eq!(
        fs::read(peer_path.join("managed")).expect("read peer target"),
        peer_before,
        "a nested authority must not mutate its sibling peer"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn unsupported_linux_filesystem_fails_before_authority_is_created() {
    let result = AuthorityRoot::<DestinationRepositoryRoot, ReadOnly>::open("/proc");
    assert!(matches!(
        result,
        Err(PathError::UnsupportedFilesystem { .. })
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn linux_identity_includes_the_mount_instance() {
    let fixture = test_directory();
    let Some(root) = read_root(fixture.path()) else {
        return;
    };
    assert_ne!(root.identity().filesystem().mount_id(), 0);
}

#[cfg(target_os = "macos")]
#[test]
fn macos_identity_case_is_capability_gated_to_apfs() {
    let fixture = test_directory();
    let Some(root) = read_root(fixture.path()) else {
        return;
    };
    assert_eq!(
        root.identity().filesystem().kind(),
        FilesystemKind::MacOsApfs
    );
    assert_ne!(
        root.identity().filesystem().mount_id(),
        0,
        "macOS authority identity must include a real mount identity"
    );
}

unsafe extern "C" {
    fn mkfifo(path: *const std::ffi::c_char, mode: u32) -> i32;
}

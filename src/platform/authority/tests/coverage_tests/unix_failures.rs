// Focused deterministic Unix authority failure and race coverage.

use super::capability;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod supported_unix {
    use super::super::super::{
        AuthorityRoot, DestinationRepositoryRoot, Mutate, MutationIntent, ObjectClass, PathError,
        ReadOnly, RelativePath,
    };
    use super::capability;
    use std::{
        fs,
        path::Path,
        sync::{Arc, Barrier},
        thread,
    };
    use tempfile::{Builder, TempDir};

    fn test_directory() -> TempDir {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
        fs::create_dir_all(&base).expect("create test base");
        let fixture = Builder::new()
            .prefix("authority-unix-failures-")
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

    #[test]
    fn read_missing_roots_leaves_and_parents_fail_with_exact_variants() {
        let fixture = test_directory();

        let missing_root = fixture.path().join("missing-root");
        assert!(matches!(
            AuthorityRoot::<DestinationRepositoryRoot, ReadOnly>::open(&missing_root),
            Err(PathError::NotFound { path }) if path == "missing-root"
        ));

        let missing_parent_root = fixture.path().join("missing-parent").join("root");
        assert!(matches!(
            AuthorityRoot::<DestinationRepositoryRoot, ReadOnly>::open(&missing_parent_root),
            Err(PathError::NotFound { path }) if path == "missing-parent"
        ));

        let Some(root) = read_root(fixture.path()) else {
            return;
        };
        let missing_leaf = RelativePath::parse("missing-leaf").expect("parse missing leaf");
        assert!(matches!(
            root.resolve_read(&missing_leaf, ObjectClass::RegularFile),
            Err(PathError::NotFound { path }) if path == "missing-leaf"
        ));

        let missing_parent =
            RelativePath::parse("missing-parent/leaf").expect("parse missing parent");
        assert!(matches!(
            root.resolve_read(&missing_parent, ObjectClass::RegularFile),
            Err(PathError::NotFound { path }) if path == "missing-parent/leaf"
        ));

        fs::write(fixture.path().join("regular"), b"payload").expect("write regular file");
        fs::create_dir(fixture.path().join("directory")).expect("create directory");

        let regular = RelativePath::parse("regular").expect("parse regular file");
        assert!(matches!(
            root.resolve_read(&regular, ObjectClass::Directory),
            Err(PathError::UnsupportedObject {
                path,
                expected: ObjectClass::Directory,
            }) if path == "regular"
        ));

        let directory = RelativePath::parse("directory").expect("parse directory");
        assert!(matches!(
            root.resolve_read(&directory, ObjectClass::RegularFile),
            Err(PathError::UnsupportedObject {
                path,
                expected: ObjectClass::RegularFile,
            }) if path == "directory"
        ));

        root.resolve_read(&regular, ObjectClass::Any)
            .expect("Any accepts regular files");
        root.resolve_read(&directory, ObjectClass::Any)
            .expect("Any accepts directories");
    }

    #[test]
    fn mutation_missing_targets_cover_each_intent_and_root_relative_rejection() {
        let fixture = test_directory();
        let Some(root) = mutation_root(fixture.path()) else {
            return;
        };
        let intents = [
            MutationIntent::CreateExclusive,
            MutationIntent::Replace,
            MutationIntent::Append,
            MutationIntent::Remove,
            MutationIntent::Rename,
        ];

        let missing_leaf = RelativePath::parse("missing-leaf").expect("parse missing leaf");
        for intent in intents {
            match root.resolve_mutation(&missing_leaf, intent) {
                Ok(target) if intent == MutationIntent::CreateExclusive => {
                    assert_eq!(target.intent(), intent);
                    assert!(target.identity().is_none());
                }
                Err(PathError::NotFound { path }) => {
                    assert_ne!(intent, MutationIntent::CreateExclusive);
                    assert_eq!(path, "missing-leaf");
                }
                _ => panic!("unexpected missing-leaf result for {intent:?}"),
            }
        }

        let missing_parent =
            RelativePath::parse("missing-parent/leaf").expect("parse missing parent");
        for intent in intents {
            assert!(matches!(
                root.resolve_mutation(&missing_parent, intent),
                Err(PathError::NotFound { path }) if path == "missing-parent/leaf"
            ));
        }

        for intent in intents {
            assert!(matches!(
                root.resolve_mutation(&RelativePath::root(), intent),
                Err(PathError::UnsupportedObject {
                    path,
                    expected: ObjectClass::RegularFile,
                }) if path.is_empty()
            ));
        }
    }

    #[test]
    fn mutation_directory_and_symlink_leaves_fail_closed() {
        let fixture = test_directory();
        let Some(root) = mutation_root(fixture.path()) else {
            return;
        };

        let directory_path = fixture.path().join("directory");
        fs::create_dir(&directory_path).expect("create directory leaf");
        let directory = RelativePath::parse("directory").expect("parse directory leaf");
        assert!(matches!(
            root.resolve_mutation(&directory, MutationIntent::Replace),
            Err(PathError::UnsupportedObject {
                path,
                expected: ObjectClass::RegularFile,
            }) if path == "directory"
        ));

        let real_file = fixture.path().join("real-file");
        fs::write(&real_file, b"payload").expect("write symlink target");
        let symlink_path = fixture.path().join("symlink-leaf");
        std::os::unix::fs::symlink(&real_file, &symlink_path)
            .expect("create symlink mutation leaf");
        let symlink = RelativePath::parse("symlink-leaf").expect("parse symlink leaf");
        assert!(matches!(
            root.resolve_mutation(&symlink, MutationIntent::Replace),
            Err(PathError::LinkLikeObject { path }) if path == "symlink-leaf"
        ));
    }

    #[test]
    fn root_identity_only_replacement_is_rejected_before_use() {
        let fixture = test_directory();
        let authority = fixture.path().join("authority");
        let moved_authority = fixture.path().join("moved-authority");
        fs::create_dir(&authority).expect("create authority root");
        fs::write(authority.join("target"), b"original-root-content")
            .expect("write original root target");

        let Some(root) = mutation_root(&authority) else {
            return;
        };
        let target = root
            .resolve_mutation(
                &RelativePath::parse("target").expect("parse target"),
                MutationIntent::Replace,
            )
            .expect("resolve target before root replacement");

        let barrier = Arc::new(Barrier::new(2));
        let attacker_barrier = Arc::clone(&barrier);
        let attacker_authority = authority.clone();
        let attacker_moved_authority = moved_authority.clone();
        let attacker = thread::spawn(move || {
            attacker_barrier.wait();
            fs::rename(&attacker_authority, &attacker_moved_authority)
                .expect("move original authority root");
            fs::create_dir(&attacker_authority).expect("replace root with ordinary directory");
            fs::write(
                attacker_authority.join("target"),
                b"replacement-root-content",
            )
            .expect("write replacement root target");
            attacker_barrier.wait();
        });

        barrier.wait();
        barrier.wait();
        attacker.join().expect("root replacement completes");

        assert!(matches!(
            target.into_file(),
            Err(PathError::ConcurrentReplacement { path, reason })
                if path == "target" && reason.contains("authority root identity changed")
        ));
        assert_eq!(
            fs::read(moved_authority.join("target")).expect("read moved root target"),
            b"original-root-content"
        );
        assert_eq!(
            fs::read(authority.join("target")).expect("read replacement root target"),
            b"replacement-root-content"
        );
    }

    #[test]
    fn ancestor_identity_only_replacement_is_rejected_before_use() {
        let fixture = test_directory();
        let authority = fixture.path().join("authority");
        let ancestor = authority.join("ancestor");
        let moved_ancestor = fixture.path().join("moved-ancestor");
        fs::create_dir(&authority).expect("create authority root");
        fs::create_dir(&ancestor).expect("create authority ancestor");
        fs::write(ancestor.join("target"), b"original-ancestor-content")
            .expect("write original ancestor target");

        let Some(root) = mutation_root(&authority) else {
            return;
        };
        let target = root
            .resolve_mutation(
                &RelativePath::parse("ancestor/target").expect("parse target"),
                MutationIntent::Replace,
            )
            .expect("resolve target before ancestor replacement");

        let barrier = Arc::new(Barrier::new(2));
        let attacker_barrier = Arc::clone(&barrier);
        let attacker_ancestor = ancestor.clone();
        let attacker_moved_ancestor = moved_ancestor.clone();
        let attacker = thread::spawn(move || {
            attacker_barrier.wait();
            fs::rename(&attacker_ancestor, &attacker_moved_ancestor)
                .expect("move original authority ancestor");
            fs::create_dir(&attacker_ancestor).expect("replace ancestor with ordinary directory");
            fs::write(
                attacker_ancestor.join("target"),
                b"replacement-ancestor-content",
            )
            .expect("write replacement ancestor target");
            attacker_barrier.wait();
        });

        barrier.wait();
        barrier.wait();
        attacker.join().expect("ancestor replacement completes");

        assert!(matches!(
            target.into_file(),
            Err(PathError::ConcurrentReplacement { path, reason })
                if path == "ancestor/target"
                    && reason.contains("authority ancestor identity changed")
        ));
        assert_eq!(
            fs::read(moved_ancestor.join("target")).expect("read moved ancestor target"),
            b"original-ancestor-content"
        );
        assert_eq!(
            fs::read(ancestor.join("target")).expect("read replacement ancestor target"),
            b"replacement-ancestor-content"
        );
    }

    #[test]
    fn leaf_identity_only_replacement_is_rejected_before_use() {
        let fixture = test_directory();
        let authority = fixture.path().join("authority");
        let original_target = authority.join("target");
        let moved_target = fixture.path().join("moved-target");
        fs::create_dir(&authority).expect("create authority root");
        fs::write(&original_target, b"original-leaf-content").expect("write original leaf");

        let Some(root) = mutation_root(&authority) else {
            return;
        };
        let target = root
            .resolve_mutation(
                &RelativePath::parse("target").expect("parse target"),
                MutationIntent::Replace,
            )
            .expect("resolve target before leaf replacement");

        let barrier = Arc::new(Barrier::new(2));
        let attacker_barrier = Arc::clone(&barrier);
        let attacker_original_target = original_target.clone();
        let attacker_moved_target = moved_target.clone();
        let attacker = thread::spawn(move || {
            attacker_barrier.wait();
            fs::rename(&attacker_original_target, &attacker_moved_target)
                .expect("move original authority leaf");
            fs::write(&attacker_original_target, b"replacement-leaf-content")
                .expect("replace leaf with ordinary file");
            attacker_barrier.wait();
        });

        barrier.wait();
        barrier.wait();
        attacker.join().expect("leaf replacement completes");

        assert!(matches!(
            target.into_file(),
            Err(PathError::ConcurrentReplacement { path, reason })
                if path == "target" && reason.contains("authority leaf identity changed")
        ));
        assert_eq!(
            fs::read(moved_target).expect("read moved original leaf"),
            b"original-leaf-content"
        );
        assert_eq!(
            fs::read(original_target).expect("read replacement leaf"),
            b"replacement-leaf-content"
        );
    }
}

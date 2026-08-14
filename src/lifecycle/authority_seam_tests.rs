//! Authority seam contracts: the destination managed consumer and the
//! verifier process consumer accept only the declared typed root with
//! explicit read or mutation capability and reject aliases, links,
//! mounts, and invalid relative forms before any effect.

#![allow(dead_code, unused_imports)]

use crate::lifecycle::verifier_confinement::confine_verifier;
use crate::platform::{
    DestinationRepositoryRoot, MutationIntent, RelativePath, open_mutation_root, open_read_root,
};
use std::{fs, path::Path};

fn harness_root(name: &str) -> tempfile::TempDir {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("fixture base");
    tempfile::Builder::new()
        .prefix(name)
        .tempdir_in(&base)
        .expect("fixture")
}

#[test]
fn the_destination_managed_consumer_accepts_only_the_typed_root_with_mutation_capability() {
    let fixture = harness_root("seam-destination-");
    let destination = fixture.path().join("destination");
    fs::create_dir_all(&destination).expect("destination");
    // The consumer opens the mutation root through the typed
    // DestinationRepositoryRoot with the explicit mutation capability.
    fs::write(destination.join("managed.txt"), "v1\n").expect("managed file");
    let authority =
        open_mutation_root::<DestinationRepositoryRoot>(&destination).expect("typed mutation root");
    let relative = RelativePath::parse("managed.txt").expect("relative");
    let target = authority
        .resolve_mutation(&relative, MutationIntent::Replace)
        .expect("resolve");
    assert!(
        authority.display_path().as_path().starts_with(&destination),
        "the mutation target stays inside the destination"
    );
    drop(target);
    // A read capability is a different, explicit capability.
    let read_root =
        open_read_root::<DestinationRepositoryRoot>(&destination).expect("typed read root");
    assert!(read_root.display_path().as_path().starts_with(&destination));
}

#[test]
fn invalid_relative_forms_are_rejected_before_effect() {
    let fixture = harness_root("seam-invalid-forms-");
    let destination = fixture.path().join("destination");
    fs::create_dir_all(&destination).expect("destination");
    // Parent traversal and absolute forms never become targets.
    assert!(RelativePath::parse("../escape.txt").is_err());
    assert!(RelativePath::parse("/etc/passwd").is_err());
    // The consumer refuses to resolve the root form as a mutation
    // target (it is not a file object).
    let authority =
        open_mutation_root::<DestinationRepositoryRoot>(&destination).expect("typed root");
    let root_form = RelativePath::root();
    assert!(
        authority
            .resolve_mutation(&root_form, MutationIntent::Replace)
            .is_err()
    );
    // Nothing was written.
    assert!(!destination.join("escape.txt").exists());
    assert!(!destination.join("passwd").exists());
}

#[test]
fn aliases_and_links_are_rejected_before_effect() {
    let fixture = harness_root("seam-aliases-");
    let destination = fixture.path().join("destination");
    fs::create_dir_all(&destination).expect("destination");
    let outside = fixture.path().join("outside-secret.txt");
    fs::write(&outside, "secret").expect("outside");
    // A symlink at the managed path is an alias: the mutation consumer
    // must refuse it before any effect.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, destination.join("managed.txt")).expect("symlink");
    }
    let authority =
        open_mutation_root::<DestinationRepositoryRoot>(&destination).expect("typed root");
    let relative = RelativePath::parse("managed.txt").expect("relative");
    let result = authority.resolve_mutation(&relative, MutationIntent::Replace);
    assert!(result.is_err(), "the alias is rejected");
    // The outside file is untouched.
    assert_eq!(fs::read_to_string(&outside).expect("outside"), "secret");
}

#[test]
fn the_verifier_consumer_accepts_only_the_typed_destination_root_read_only() {
    let fixture = harness_root("seam-verifier-");
    let destination = fixture.path().join("destination");
    fs::create_dir_all(&destination).expect("destination");
    // The verifier consumer is confined through the typed root with the
    // explicit read capability; ephemeral artifacts must be valid
    // root-relative paths with no duplicates.
    let confinement =
        confine_verifier(&destination, &["out.txt".to_owned()], false).expect("confinement");
    assert!(
        confinement
            .destination
            .display_path()
            .as_path()
            .starts_with(&destination),
        "the verifier is confined to the destination"
    );
    // Invalid relative forms and duplicates are refused before
    // execution.
    let traversal = confine_verifier(&destination, &["../escape.txt".to_owned()], false);
    assert!(traversal.is_err(), "{traversal:?}");
    let duplicate = confine_verifier(
        &destination,
        &["out.txt".to_owned(), "out.txt".to_owned()],
        false,
    );
    assert!(duplicate.is_err(), "{duplicate:?}");
}

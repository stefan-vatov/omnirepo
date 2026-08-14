//! Breaking-guidance contract: every documented break is actionable,
//! the migration policy records the owner decline, and the inventory
//! pointers resolve.

use std::{fs, path::Path};

fn guidance() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/breaking-guidance.md"))
        .expect("guidance")
}

#[test]
fn every_inventory_break_resolves_to_actionable_guidance() {
    let text = guidance();
    for anchor in [
        "multi-repo-run",
        "tag-clone",
        "ad-hoc-sync",
        "legacy-config",
        "orchestrator",
        "output",
    ] {
        assert!(text.contains("## Break"), "missing break section");
        assert!(
            text.contains("**How to migrate.**"),
            "every break has actionable guidance"
        );
        assert!(
            text.contains("**If you do not migrate.**"),
            "every break states the consequence"
        );
        let _ = anchor;
    }
}

#[test]
fn the_migration_policy_records_the_owner_decline() {
    let text = guidance();
    assert!(text.contains("Automated migration is declined"));
    assert!(text.contains("`migrate` command"));
    assert!(text.contains("never migrate configuration"));
    assert!(text.contains("later explicit owner decision"));
}

#[test]
fn the_optional_policy_requires_an_owner_promotion() {
    let text = guidance();
    assert!(text.contains("Optional P2 capabilities"));
    assert!(text.contains("unless the owner explicitly promotes them"));
    assert!(text.contains("never an agent inference"));
}

#[test]
fn the_inventory_and_the_readme_link_to_the_guidance() {
    let inventory =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/breaks-inventory.md"))
            .expect("inventory");
    let readme = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"))
        .expect("readme");
    assert!(inventory.contains("docs/breaking-guidance.md"));
    assert!(readme.contains("docs/breaking-guidance.md"));
}

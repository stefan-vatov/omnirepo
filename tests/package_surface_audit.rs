//! The package-surface audit: help text, Cargo metadata, package
//! files, doc links, and the executable surface stay constitutional.

use std::{fs, path::Path};

#[test]
fn the_cargo_metadata_describes_the_constitutional_product() {
    let manifest = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .expect("manifest");
    assert!(manifest.contains("description = \"Constitutional synchronization"));
    assert!(
        !manifest.contains("managing multiple Git repositories"),
        "the stale general-tool description is gone"
    );
    for keyword in ["synchronization", "convergence", "constitution"] {
        assert!(manifest.contains(keyword), "missing keyword {keyword}");
    }
}

#[test]
fn the_readme_doc_links_resolve_within_the_repository() {
    let readme = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"))
        .expect("readme");
    for link in [
        "docs/quickstart.md",
        "docs/breaking-guidance.md",
        "docs/breaks-inventory.md",
    ] {
        assert!(readme.contains(link), "missing link {link}");
        assert!(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(link).exists(),
            "broken link {link}"
        );
    }
}

#[test]
fn the_docs_are_development_content_and_stay_out_of_the_package() {
    // The package-surface contract forbids development content in the
    // published package; the docs are development content.
    let manifest = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .expect("manifest");
    assert!(
        manifest.contains("\"docs\","),
        "docs must stay excluded from the package"
    );
}

#[test]
fn the_executable_surface_matches_the_documented_commands() {
    let help = std::process::Command::new(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/omnirepo"),
    )
    .arg("--help")
    .output()
    .expect("help");
    assert!(help.status.success());
    let text = String::from_utf8(help.stdout).expect("utf8");
    for command in ["sync", "setup", "validate"] {
        assert!(text.contains(command), "missing {command}");
    }
}

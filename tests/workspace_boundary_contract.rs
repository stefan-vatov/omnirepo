//! Contract tests for the private developer-tool workspace boundary.
//!
//! This starts as a RED test for `omni-beads-rust-tooling-3eic.2`.  The
//! assertions describe the smallest supported topology: the root package is
//! the publishable product binary, while `tools/omnirepo-dev` is a private
//! library-plus-CLI crate for repository automation.  The private crate must
//! never become a runtime dependency of the product.

use std::{fs, path::PathBuf, process::Command};
use toml_edit::{DocumentMut, Item as TomlItem};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn cargo_metadata() -> String {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps", "--locked"])
        .current_dir(repository_root())
        .output()
        .expect("run locked cargo metadata");

    assert!(
        output.status.success(),
        "cargo metadata failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("cargo metadata is UTF-8")
}

fn package_section<'a>(metadata: &'a str, package_name: &str) -> &'a str {
    let marker = format!("\"name\":\"{package_name}\",\"version\"");
    let start = metadata
        .find(&marker)
        .unwrap_or_else(|| panic!("cargo metadata is missing package {package_name:?}"));
    let targets = metadata[start..]
        .find(",\"targets\"")
        .map(|offset| start + offset)
        .unwrap_or(start);
    let end = metadata[targets..]
        .find("},{\"name\":\"")
        .map(|offset| targets + offset)
        .unwrap_or(metadata.len());
    &metadata[start..end]
}

fn package_listing() -> String {
    let output = Command::new("cargo")
        .args(["package", "--list", "--allow-dirty"])
        .current_dir(repository_root())
        .output()
        .expect("run cargo package listing");

    assert!(
        output.status.success(),
        "cargo package --list failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("cargo package listing is UTF-8")
}

fn dependency_tables(document: &DocumentMut) -> impl Iterator<Item = &toml_edit::Table> {
    ["dependencies", "dev-dependencies", "build-dependencies"]
        .into_iter()
        .filter_map(|section_name| document.get(section_name).and_then(TomlItem::as_table))
}

fn assert_no_dependency(manifest: &str, forbidden_name: &str) {
    let document = manifest
        .parse::<DocumentMut>()
        .expect("Cargo manifest must be valid TOML");
    for section in dependency_tables(&document) {
        assert!(
            section.get(forbidden_name).is_none(),
            "manifest must not make the product depend on private crate {forbidden_name:?}"
        );
    }
}

fn assert_no_path_dependencies(manifest: &str) {
    let document = manifest
        .parse::<DocumentMut>()
        .expect("Cargo manifest must be valid TOML");
    for section in dependency_tables(&document) {
        for (dependency_name, dependency) in section.iter() {
            let has_path = dependency
                .as_inline_table()
                .and_then(|table| table.get("path"))
                .is_some()
                || dependency
                    .as_table()
                    .and_then(|table| table.get("path"))
                    .is_some();
            assert!(
                !has_path,
                "dependency {dependency_name:?} must not use a path dependency"
            );
        }
    }
}

#[test]
fn workspace_has_one_publishable_product_and_private_tool_members() {
    let root = repository_root();
    let metadata = cargo_metadata();
    let root_manifest = fs::read_to_string(root.join("Cargo.toml")).expect("read root manifest");
    let tool_manifest =
        fs::read_to_string(root.join("tools/omnirepo-dev/Cargo.toml")).expect("read tool manifest");
    let support_manifest = fs::read_to_string(root.join("tools/omnirepo-test-support/Cargo.toml"))
        .expect("read test-support manifest");
    let root_package = package_section(&metadata, "omnirepo");
    let tool_package = package_section(&metadata, "omnirepo-dev");
    let support_package = package_section(&metadata, "omnirepo-test-support");

    assert!(
        root_manifest.contains("[workspace]"),
        "workspace metadata must be explicit"
    );
    assert!(
        root_manifest
            .contains("members = [\"tools/omnirepo-dev\", \"tools/omnirepo-test-support\"]"),
        "the private developer and test-support crates must be explicit workspace members"
    );
    assert!(
        root_manifest.contains("resolver = \"3\""),
        "Rust 2024 workspace resolver must be explicit"
    );
    assert!(root_manifest.contains("edition = \"2024\""));
    assert!(root_manifest.contains("rust-version = \"1.86\""));
    assert!(tool_manifest.contains("name = \"omnirepo-dev\""));
    assert!(tool_manifest.contains("publish = false"));
    assert!(tool_manifest.contains("edition.workspace = true"));
    assert!(tool_manifest.contains("rust-version.workspace = true"));
    assert!(root_package.contains("\"name\":\"omnirepo\""));
    assert!(tool_package.contains("\"name\":\"omnirepo-dev\""));
    assert!(tool_package.contains("\"edition\":\"2024\""));
    assert!(tool_package.contains("\"rust_version\":\"1.86\""));
    assert!(support_manifest.contains("name = \"omnirepo-test-support\""));
    assert!(support_manifest.contains("publish = false"));
    assert!(support_manifest.contains("edition.workspace = true"));
    assert!(support_manifest.contains("rust-version.workspace = true"));
    assert!(support_package.contains("\"name\":\"omnirepo-test-support\""));
    assert!(support_package.contains("\"edition\":\"2024\""));
    assert!(support_package.contains("\"rust_version\":\"1.86\""));
    assert!(
        metadata.contains("tools/omnirepo-dev"),
        "cargo metadata must include the private tool member"
    );
    assert!(
        metadata.contains("tools/omnirepo-test-support"),
        "cargo metadata must include the private test-support member"
    );
    assert!(
        tool_package.contains("\"publish\":[]") || tool_manifest.contains("publish = false"),
        "the private developer tool must be non-publishable"
    );
}

#[test]
fn private_tool_has_testable_library_and_thin_cli_targets() {
    let metadata = cargo_metadata();
    let tool_package = package_section(&metadata, "omnirepo-dev");

    assert!(
        tool_package.contains("\"kind\":[\"lib\"]"),
        "developer tooling behavior must live behind a testable library target"
    );
    assert!(
        tool_package.contains("\"kind\":[\"bin\"]"),
        "developer tooling must expose a thin CLI target"
    );
    assert!(
        repository_root()
            .join("tools/omnirepo-dev/src/lib.rs")
            .is_file(),
        "developer-tool library seam is missing"
    );
    assert!(
        repository_root()
            .join("tools/omnirepo-dev/src/main.rs")
            .is_file(),
        "developer-tool CLI seam is missing"
    );
}

#[test]
fn product_and_tool_dependency_edges_are_one_way() {
    let root = repository_root();
    let root_manifest = fs::read_to_string(root.join("Cargo.toml")).expect("read root manifest");
    let tool_manifest =
        fs::read_to_string(root.join("tools/omnirepo-dev/Cargo.toml")).expect("read tool manifest");

    assert_no_dependency(&root_manifest, "omnirepo-dev");
    assert_no_dependency(&tool_manifest, "omnirepo");
    assert_no_path_dependencies(&root_manifest);
}

#[test]
fn product_package_excludes_private_tooling_and_tracker_data() {
    let listing = package_listing();

    for forbidden in [
        "tools/omnirepo-dev/",
        "tools/omnirepo-test-support/",
        ".beads/",
        "canon/",
        ".codex/",
        ".claude/",
        "coverage/",
    ] {
        assert!(
            !listing.lines().any(|line| line.contains(forbidden)),
            "published product package must exclude {forbidden:?}; listing:\n{listing}"
        );
    }
}

#[test]
fn product_package_declares_coverage_and_instrumentation_exclusions() {
    let root = repository_root();
    let manifest = fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    assert!(
        manifest.lines().any(|line| line.trim() == "\"coverage\","),
        "path=Cargo.toml:package.exclude.coverage; coverage/ must be explicitly excluded from the product package"
    );

    let gitignore = fs::read_to_string(root.join(".gitignore")).expect("read .gitignore");
    assert!(
        gitignore.lines().any(|line| line.trim() == "*.profraw"),
        "path=.gitignore:*.profraw; LLVM raw-profile residue must be ignored at every depth"
    );
}

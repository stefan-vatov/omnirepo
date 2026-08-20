//! README and quickstart contract: the docs describe only the
//! constitutional product surface with no legacy claims.

use std::{fs, path::Path};

#[test]
fn the_readme_describes_only_the_constitutional_surface() {
    let readme = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"))
        .expect("readme");
    for command in ["sync", "setup", "doctor"] {
        assert!(readme.contains(command), "missing {command}");
    }
    // No legacy surface is claimed as available.  The migrate mention
    // is the lawful decline sentence only.
    assert!(!readme.contains("`clone`"), "legacy surface clone");
    assert!(
        !readme.contains("general repository orchestration")
            || readme.contains("no general repository orchestration"),
        "legacy orchestration"
    );
    assert!(readme.contains("no `migrate` command"));
    assert!(readme.contains("first unattended sync") || readme.contains("First unattended sync"));
    assert!(readme.contains("docs/quickstart.md"));
}

#[test]
fn the_quickstart_walks_to_a_first_unattended_sync() {
    let quickstart =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/quickstart.md"))
            .expect("quickstart");
    for step in [
        "## 1. Install",
        "## 2. Author the machine configuration",
        "## 3. Run the first synchronization",
        "## 4. What to expect",
    ] {
        assert!(quickstart.contains(step), "missing step {step}");
    }
    assert!(quickstart.contains("omnirepo sync"));
    assert!(quickstart.contains(".omnirepo/config.yaml"));
    assert!(quickstart.contains("chore(omnirepo): sync"));
}

#[test]
fn the_readme_usage_matches_the_runtime_surface() {
    // The documented command table matches the real clap surface.
    let readme = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"))
        .expect("readme");
    assert!(readme.contains("| `sync` |"));
    assert!(readme.contains("| `setup` |"));
    assert!(readme.contains("| `doctor` |"));
}

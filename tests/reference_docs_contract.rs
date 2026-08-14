//! Reference-docs contract: the published references match the runtime
//! surface and the canonical contracts.

use std::{fs, path::Path};

fn reference() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/reference.md"))
        .expect("reference")
}

#[test]
fn the_cli_reference_matches_the_runtime_surface_and_exit_map() {
    let text = reference();
    for command in ["`omnirepo sync`", "`omnirepo setup", "`omnirepo validate`"] {
        assert!(text.contains(command), "missing {command}");
    }
    for (code, meaning) in [
        ("| 0 |", "Success"),
        ("| 2 |", "Invocation"),
        ("| 3 |", "Some repositories"),
        ("| 4 |", "Every selected"),
        ("| 5 |", "Durable-record"),
        ("| 130 |", "User cancellation"),
    ] {
        assert!(text.contains(code), "missing exit {code}");
        assert!(text.contains(meaning), "missing meaning {meaning}");
    }
    assert!(text.contains("--output human|json"));
}

#[test]
fn the_configuration_reference_names_the_three_canonical_paths() {
    let text = reference();
    for path in [
        "<HOME>/.omnirepo/config.yaml",
        "<source-root>/.omnirepo/source.yaml",
        "<destination-root>/.omnirepo.yaml",
    ] {
        assert!(text.contains(path), "missing {path}");
    }
    assert!(text.contains("version: 1"));
    assert!(text.contains("max_repositories: 4"));
    assert!(text.contains("max_child_work: 8"));
}

#[test]
fn the_delimiter_reference_covers_the_registered_formats() {
    let text = reference();
    for format in [
        "yaml / toml / shell",
        "json / javascript / typescript",
        "markdown / html",
        "python",
        "rust",
    ] {
        assert!(text.contains(&format!("| {format} |")), "missing {format}");
    }
    assert!(text.contains("# omnirepo-start"));
    assert!(text.contains("<!-- omnirepo-start -->"));
}

#[test]
fn the_operation_reference_describes_the_full_lifecycle() {
    let text = reference();
    for step in [
        "Invocation",
        "Catalog",
        "Plans",
        "Pass",
        "Repair",
        "Finalize",
    ] {
        assert!(text.contains(&format!("**{step}**")), "missing {step}");
    }
    assert!(text.contains("chore(omnirepo): sync managed content"));
    assert!(text.contains("never promotes a lower source"));
    assert!(text.contains("creates no commit"));
}

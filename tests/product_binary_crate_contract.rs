use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn cargo_manifest() -> String {
    fs::read_to_string(root().join("Cargo.toml")).expect("read root Cargo.toml")
}

fn metadata() -> String {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps", "--locked"])
        .current_dir(root())
        .output()
        .expect("run locked cargo metadata");

    assert!(
        output.status.success(),
        "cargo metadata failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("cargo metadata is UTF-8")
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.match_indices(needle).count()
}

fn json_array_end(document: &str, array_start: usize) -> usize {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (offset, character) in document[array_start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }

        match character {
            '"' => in_string = true,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return array_start + offset + 1;
                }
            }
            _ => {}
        }
    }

    panic!("cargo metadata contains an unterminated JSON array")
}

fn root_targets(metadata: &str) -> &str {
    let package_start = metadata
        .find("\"name\":\"omnirepo\"")
        .expect("metadata contains the omnirepo package");
    let targets_key = "\"targets\":[";
    let targets_key_start = metadata[package_start..]
        .find(targets_key)
        .map(|offset| package_start + offset)
        .expect("omnirepo package contains a targets array");
    let array_start = targets_key_start + targets_key.len() - 1;
    let array_end = json_array_end(metadata, array_start);
    &metadata[array_start..array_end]
}

fn dependencies_section(manifest: &str) -> &str {
    let section = "[dependencies]";
    let start = manifest
        .find(section)
        .expect("root manifest contains [dependencies]")
        + section.len();
    let end = manifest[start..]
        .find("\n[")
        .map(|offset| start + offset)
        .unwrap_or(manifest.len());
    &manifest[start..end]
}

fn binary_path() -> PathBuf {
    option_env!("CARGO_BIN_EXE_omnirepo")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            panic!(
                "Cargo did not provide CARGO_BIN_EXE_omnirepo for this current-build integration test; refusing to execute any target/debug fallback"
            )
        })
}

fn binary(args: &[&str]) -> Output {
    let path = binary_path();
    assert!(
        path.is_file(),
        "Cargo-selected current-build omnirepo executable is unavailable at {}; refusing to execute any stale target/debug fallback",
        path.display()
    );
    Command::new(&path)
        .args(args)
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "failed to run Cargo-selected current-build omnirepo executable at {} with args {args:?}: {error}",
                path.display()
            )
        })
}

fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(directory).expect("read source directory");
    for entry in entries {
        let path = entry.expect("read source entry").path();
        if path.is_dir() {
            collect_files(&path, files);
        } else {
            files.push(path);
        }
    }
}

#[test]
fn root_package_exposes_one_binary_target_and_no_library_target() {
    let document = metadata();
    let targets = root_targets(&document);

    assert_eq!(
        count_occurrences(targets, "\"kind\":[\"bin\"]"),
        1,
        "the root product package must expose exactly one binary target"
    );
    assert_eq!(
        count_occurrences(targets, "\"kind\":[\"lib\"]"),
        0,
        "the root product package must not expose a library target"
    );
    assert!(
        targets.contains("\"name\":\"omnirepo\""),
        "the sole root binary target must be named omnirepo"
    );
}

#[test]
fn main_is_the_private_composition_root_without_legacy_exports() {
    let repository = root();
    let manifest = cargo_manifest();
    let main_path = repository.join("src/main.rs");
    let main = fs::read_to_string(&main_path).unwrap_or_default();

    assert!(
        main_path.is_file(),
        "src/main.rs must be the composition root"
    );
    assert!(!repository.join("src/bin/main.rs").exists());
    assert!(!repository.join("src/lib.rs").exists());
    assert!(!manifest.contains("\n[lib]"));
    assert!(!manifest.contains("omnirepo_lib"));
    assert!(main.contains("fn main"));
    assert!(
        main.lines()
            .any(|line| line.trim_start().starts_with("mod ")),
        "src/main.rs must declare at least one private product module"
    );
    assert!(
        !main
            .lines()
            .any(|line| line.trim_start().starts_with("pub mod ")),
        "the binary composition root must not publish modules"
    );

    let mut source_files = Vec::new();
    collect_files(&repository.join("src"), &mut source_files);
    let forbidden_markers = [
        "omnirepo_lib",
        "pub mod config",
        "pub mod util",
        "load_config",
        "GlobalConfig",
        "RepoConfig",
        "GLOBAL_CONFIG",
    ];
    for path in source_files {
        let contents = fs::read_to_string(&path).expect("read product source file");
        for marker in forbidden_markers {
            assert!(
                !contents.contains(marker),
                "forbidden legacy marker {marker:?} remains in {}",
                path.display()
            );
        }
    }
}

#[test]
fn product_runtime_dependencies_have_no_private_path_edges() {
    let manifest = cargo_manifest();
    let dependencies = dependencies_section(&manifest);

    assert!(
        !dependencies.lines().any(|line| line.contains("path =")),
        "the product runtime dependency section must not contain path dependencies"
    );
    assert!(
        !dependencies.contains("omnirepo-dev") && !dependencies.contains("test-support"),
        "private developer and test-support crates must not be runtime dependencies"
    );
}

#[test]
fn cli_help_version_and_forbidden_commands_remain_stable() {
    let help = binary(&["--help"]);
    assert!(
        help.status.success(),
        "omnirepo --help failed with status {:?}; stdout={:?}; stderr={:?}",
        help.status,
        String::from_utf8_lossy(&help.stdout),
        String::from_utf8_lossy(&help.stderr)
    );
    let help_text = String::from_utf8(help.stdout).expect("help is UTF-8");
    for declared in ["sync", "setup", "doctor"] {
        assert!(
            help_text.contains(declared),
            "help must declare the constitutional command {declared:?}"
        );
    }
    assert!(
        help_text.contains("--output"),
        "help must declare the machine-readable output flag"
    );
    assert!(
        help_text
            .contains("Machine configuration: <HOME>/.omnirepo/config.yaml (YAML version: 1)."),
        "help must keep the machine configuration pointer"
    );

    let version = binary(&["--version"]);
    assert!(
        version.status.success(),
        "omnirepo --version failed with status {:?}; stdout={:?}; stderr={:?}",
        version.status,
        String::from_utf8_lossy(&version.stdout),
        String::from_utf8_lossy(&version.stderr)
    );
    assert_eq!(
        String::from_utf8(version.stdout).expect("version is UTF-8"),
        format!("omnirepo {}\n", env!("CARGO_PKG_VERSION"))
    );

    // Legacy general surfaces and the first-release no-migrate decision stay
    // rejected; the constitutional commands are recognized and fail closed
    // until the lifecycle slices land (owner exit contract: 2).
    for forbidden_command in ["new", "clone", "run", "migrate"] {
        let output = binary(&[forbidden_command]);
        assert_eq!(
            output.status.code(),
            Some(2),
            "forbidden command {forbidden_command:?} must remain rejected"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand")
                || String::from_utf8_lossy(&output.stderr).contains("unexpected argument"),
            "forbidden command {forbidden_command:?} must report an argument error"
        );
    }
    for recognized in ["sync", "setup", "doctor"] {
        let output = binary(&[recognized]);
        let status = output.status.code();
        match recognized {
            "sync" => {
                // sync with no machine authority is an empty-fleet success
                // (the .27 contract) in the test environment's HOME.
                assert!(
                    status == Some(0) || status == Some(2),
                    "{recognized:?} must be a fleet run (success) or fail closed: {status:?}"
                );
            }
            "doctor" => {
                // doctor with no machine authority is a healthy empty
                // fleet; with problems it exits 2.  Either way it lands.
                assert!(
                    status == Some(0) || status == Some(2),
                    "{recognized:?} must report healthy or problems: {status:?}"
                );
            }
            _ => {
                assert_eq!(
                    status,
                    Some(2),
                    "{recognized:?} must fail closed with the invocation exit until its lifecycle lands"
                );
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains("not available in this build"),
                    "{recognized:?} must not claim landed capabilities"
                );
            }
        }
    }
}

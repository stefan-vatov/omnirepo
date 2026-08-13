use std::{
    fs,
    path::{Path, PathBuf},
};

const CRATE_DIR: &str = "tools/omnirepo-test-support";
const MODULES: &[&str] = &[
    "agent_double.rs",
    "git_double.rs",
    "lifecycle_fixture.rs",
    "network_double.rs",
    "process_double.rs",
    "recovery_control.rs",
];
const PRIVATE_TESTS: &[&str] = &[
    "fixture_layer_self_test.rs",
    "harness_usage_patterns.rs",
    "lifecycle_fixture_tests.rs",
    "process_network_git_agent_doubles.rs",
    "recovery_control_tests.rs",
];

fn repository_path(relative: impl AsRef<Path>) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

#[test]
fn private_fixture_crate_is_not_a_product_dependency() {
    let manifest = fs::read_to_string(repository_path(format!("{CRATE_DIR}/Cargo.toml")))
        .expect("test-support manifest should exist");
    assert!(manifest.contains("name = \"omnirepo-test-support\""));
    assert!(manifest.contains("publish = false"));

    let dependencies = manifest
        .split_once("[dependencies]")
        .map_or("", |(_, section)| section);
    assert!(
        !dependencies.lines().any(|line| {
            let name = line
                .trim()
                .split_once('=')
                .map_or("", |(name, _)| name.trim());
            name == "omnirepo"
        }),
        "test infrastructure must not depend on the product crate"
    );
}

#[test]
fn private_crate_owns_all_reusable_fixture_sources() {
    for module in MODULES {
        let path = repository_path(format!("{CRATE_DIR}/src/{module}"));
        assert!(
            path.is_file(),
            "fixture source is not owned by private crate: {path:?}"
        );
    }
}

#[test]
fn fixture_consumers_do_not_compile_support_by_path() {
    for consumer in PRIVATE_TESTS {
        let source = fs::read_to_string(repository_path(format!("{CRATE_DIR}/tests/{consumer}")))
            .expect("private consumer should exist");
        assert!(
            !source.contains("#[path = \"support/") && !source.contains("mod support;"),
            "consumer still compiles fixture modules by path: {consumer}"
        );
    }
}

#[test]
fn private_crate_has_no_source_path_back_to_tests() {
    let source = fs::read_to_string(repository_path(format!("{CRATE_DIR}/src/lib.rs")))
        .expect("private crate root should exist");
    assert!(
        !source.contains("tests/support"),
        "private crate must own fixture source instead of reaching into tests"
    );
}

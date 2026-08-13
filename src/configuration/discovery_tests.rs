//! Focused proof for canonical machine-config discovery and loading.

#![allow(dead_code, unused_imports)]

use super::discovery::{
    CONFIG_DIRECTORY, CONFIG_FILE_NAME, Discovery, DiscoveryError, canonical_config_path, discover,
};
use crate::configuration::{AgentKind, MachineConfiguration, SourceLocation};
use std::{fs, path::Path};

const VALID_CONFIG: &str = "version: 1
repositories:
  - id: destination-a
    path: /srv/repositories/a
    tags: [prod]
  - id: destination-b
    path: /srv/repositories/b
sources:
  - id: upstream
    location: https://example.com/repo.git
  - id: local-mirror
    location: /srv/mirrors/upstream
cache_root: /var/cache/omnirepo
concurrency:
  max_repositories: 8
  max_child_work: 16
repair:
  priority: [codex, pi]
  max_attempts: 3
";

fn fixture_home() -> tempfile::TempDir {
    tempfile::TempDir::new().expect("temporary home")
}

fn write_config(home: &Path, content: &str) {
    let directory = home.join(CONFIG_DIRECTORY);
    fs::create_dir_all(&directory).expect("create config directory");
    fs::write(directory.join(CONFIG_FILE_NAME), content).expect("write config");
}

#[test]
fn absent_config_is_a_distinct_lawful_state() {
    let home = fixture_home();
    match discover(home.path()).expect("discovery must not fail") {
        Discovery::Absent => {}
        other => panic!("expected absent, got {other:?}"),
    }
}

#[test]
fn valid_config_loads_exact_typed_values() {
    let home = fixture_home();
    write_config(home.path(), VALID_CONFIG);
    let Discovery::Present(config) = discover(home.path()).expect("valid config loads") else {
        panic!("expected present config");
    };
    assert_eq!(config.version().value(), 1);
    let repositories = config.repositories();
    assert_eq!(repositories.len(), 2);
    assert_eq!(repositories[0].id().as_str(), "destination-a");
    assert_eq!(repositories[0].path().as_str(), "/srv/repositories/a");
    assert_eq!(repositories[0].tags().len(), 1);
    assert_eq!(repositories[0].tags()[0].as_str(), "prod");
    assert_eq!(repositories[1].id().as_str(), "destination-b");
    assert!(repositories[1].tags().is_empty());

    let sources = config.sources();
    assert_eq!(sources.len(), 2);
    assert_eq!(sources[0].id().as_str(), "upstream");
    assert!(
        matches!(sources[0].location(), SourceLocation::Remote(url) if url == "https://example.com/repo.git")
    );
    assert_eq!(sources[1].id().as_str(), "local-mirror");
    assert!(
        matches!(sources[1].location(), SourceLocation::Local(path) if path.as_str() == "/srv/mirrors/upstream")
    );
    assert_eq!(
        config.cache_root().expect("cache root").as_str(),
        "/var/cache/omnirepo"
    );
    assert_eq!(config.concurrency().max_repositories(), 8);
    assert_eq!(config.concurrency().max_child_work(), 16);
    assert_eq!(config.repair().max_attempts(), 3);
    assert_eq!(
        config.repair().priority(),
        &[AgentKind::Codex, AgentKind::Pi]
    );
}

#[test]
fn symlink_config_is_a_typed_alias_error() {
    let home = fixture_home();
    let directory = home.path().join(CONFIG_DIRECTORY);
    fs::create_dir_all(&directory).expect("create config directory");
    let real = home.path().join("real-config.yaml");
    fs::write(&real, VALID_CONFIG).expect("write real config");
    std::os::unix::fs::symlink(&real, directory.join(CONFIG_FILE_NAME)).expect("symlink config");
    let error = discover(home.path()).expect_err("symlink config must fail");
    assert!(matches!(error, DiscoveryError::Alias { .. }), "{error:?}");
}

#[test]
fn directory_config_is_a_typed_non_regular_error() {
    let home = fixture_home();
    let directory = home.path().join(CONFIG_DIRECTORY);
    fs::create_dir_all(directory.join(CONFIG_FILE_NAME)).expect("create config directory entry");
    let error = discover(home.path()).expect_err("directory config must fail");
    assert!(
        matches!(error, DiscoveryError::NotRegular { .. }),
        "{error:?}"
    );
}

#[cfg(unix)]
#[test]
fn unreadable_config_is_a_typed_permission_error() {
    use std::os::unix::fs::PermissionsExt;
    let home = fixture_home();
    write_config(home.path(), VALID_CONFIG);
    let path = home.path().join(CONFIG_DIRECTORY).join(CONFIG_FILE_NAME);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmod 000");
    let error = discover(home.path()).expect_err("unreadable config must fail");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("restore");
    assert!(
        matches!(error, DiscoveryError::Permission { .. }),
        "{error:?}"
    );
}

#[test]
fn malformed_yaml_is_a_typed_error() {
    let home = fixture_home();
    write_config(home.path(), "version: 1\nrepositories: [unclosed\n");
    let error = discover(home.path()).expect_err("malformed config must fail");
    assert!(
        matches!(error, DiscoveryError::Malformed { .. }),
        "{error:?}"
    );
}

#[test]
fn unsupported_version_is_a_typed_error() {
    let home = fixture_home();
    write_config(home.path(), "version: 2\n");
    let error = discover(home.path()).expect_err("unsupported version must fail");
    assert!(
        matches!(error, DiscoveryError::UnsupportedVersion { version: 2, .. }),
        "{error:?}"
    );
}

#[test]
fn duplicate_repository_id_is_a_typed_invalid_error() {
    let home = fixture_home();
    write_config(
        home.path(),
        "version: 1\nrepositories:\n  - id: destination-a\n    path: /srv/a\n  - id: destination-a\n    path: /srv/b\n",
    );
    let error = discover(home.path()).expect_err("duplicate repository id must fail");
    assert!(matches!(error, DiscoveryError::Invalid { .. }), "{error:?}");
}

#[test]
fn remote_source_without_cache_root_is_rejected() {
    let home = fixture_home();
    write_config(
        home.path(),
        "version: 1\nsources:\n  - id: upstream\n    location: https://example.com/repo.git\n",
    );
    let error = discover(home.path()).expect_err("remote source without cache root must fail");
    assert!(matches!(error, DiscoveryError::Invalid { .. }), "{error:?}");
}

#[test]
fn relative_home_is_a_typed_pre_effect_error() {
    let _home = fixture_home();
    let error = discover(Path::new("relative/home")).expect_err("relative home must fail");
    assert!(
        matches!(error, DiscoveryError::HomeUnavailable { .. }),
        "{error:?}"
    );
}

#[test]
fn canonical_path_is_exact_and_never_scans() {
    let home = fixture_home();
    // An adjacent file with another extension or name must never be picked up.
    fs::create_dir_all(home.path().join(CONFIG_DIRECTORY)).expect("config directory");
    fs::write(
        home.path().join(CONFIG_DIRECTORY).join("config.yml"),
        VALID_CONFIG,
    )
    .expect("write alternate extension");
    fs::write(
        home.path().join(CONFIG_DIRECTORY).join("settings.yaml"),
        VALID_CONFIG,
    )
    .expect("write alternate name");
    match discover(home.path()).expect("discovery must not fail") {
        Discovery::Absent => {}
        other => panic!("alternate files must not be discovered, got {other:?}"),
    }
    assert_eq!(
        canonical_config_path(home.path()),
        home.path().join(CONFIG_DIRECTORY).join(CONFIG_FILE_NAME)
    );
}

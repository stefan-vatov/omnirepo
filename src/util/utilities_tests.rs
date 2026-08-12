use super::*;
use proptest::prelude::*;
use std::fs;
use tempfile::tempdir;

fn valid_config_yaml(name: &str) -> String {
    format!(
        "repositories:\n  - name: {name}\n    url: https://example.test/{name}.git\n    tags:\n      - test\n    dest: {name}\ntemplates:\n  - name: template\n    id: template-id\n    url: https://example.test/template\n    kind: File\n    dest: .\n    tags:\n      - test\n"
    )
}

#[test]
fn string_dedupe_is_stable_and_exact() {
    let input = vec![
        "second".to_owned(),
        "first".to_owned(),
        "second".to_owned(),
        "".to_owned(),
        "First".to_owned(),
        "first".to_owned(),
        "".to_owned(),
    ];

    assert_eq!(
        dedupe_vec_string(input),
        vec!["second", "first", "", "First"]
    );
}

#[test]
fn tuple_dedupe_is_stable_and_keeps_distinct_destinations() {
    let input = vec![
        ("url-a".to_owned(), "dest-a".to_owned()),
        ("url-a".to_owned(), "dest-b".to_owned()),
        ("url-a".to_owned(), "dest-a".to_owned()),
        ("url-b".to_owned(), "dest-a".to_owned()),
        ("url-a".to_owned(), "dest-b".to_owned()),
    ];

    assert_eq!(
        dedupe_vec_tuple(input),
        vec![
            ("url-a".into(), "dest-a".into()),
            ("url-a".into(), "dest-b".into()),
            ("url-b".into(), "dest-a".into()),
        ]
    );
}

#[test]
fn tag_helpers_dedupe_in_tag_and_config_order() {
    let config: Config = yaml_serde::from_str(
        "repositories:\n  - name: one\n    url: https://example.test/one.git\n    tags: [first, shared]\n    dest: one\n  - name: two\n    url: https://example.test/two.git\n    tags: [shared, second]\n    dest: two\n  - name: duplicate\n    url: https://example.test/one.git\n    tags: [second]\n    dest: one-copy\ntemplates:\n  - name: one\n    id: one-id\n    url: https://example.test/template\n    kind: File\n    dest: first\n    tags: [first]\n  - name: two\n    id: two-id\n    url: https://example.test/template\n    kind: File\n    dest: second\n    tags: [second]\n  - name: duplicate\n    id: duplicate-id\n    url: https://example.test/template\n    kind: File\n    dest: first\n    tags: [second]\n",
    )
    .unwrap();
    let manager = GlobalConfigManager::new(config);

    assert_eq!(
        get_repos_from_tags(&["first".into(), "second".into()], &manager),
        vec![
            "https://example.test/one.git",
            "https://example.test/two.git",
        ]
    );
    assert_eq!(
        get_dest_from_tags(&["first".into(), "second".into()], &manager),
        vec!["one", "two", "one-copy"]
    );
    assert_eq!(
        template_and_dest_from_tags(&["first".into(), "second".into()], &manager),
        vec![
            ("https://example.test/template".into(), "first".into()),
            ("https://example.test/template".into(), "second".into()),
        ]
    );
}

proptest! {
    #[test]
    fn string_dedupe_keeps_first_occurrence(values in prop::collection::vec("[a-zA-Z0-9]{0,8}", 0..40)) {
        let actual = dedupe_vec_string(values.clone());
        let mut expected = Vec::new();
        for value in values {
            if !expected.contains(&value) {
                expected.push(value);
            }
        }

        prop_assert_eq!(actual, expected);
    }

    #[test]
    fn tuple_dedupe_keeps_first_exact_pair(
        values in prop::collection::vec(("[a-z]{0,6}", "[a-z]{0,6}"), 0..40)
    ) {
        let actual = dedupe_vec_tuple(values.clone());
        let mut expected = Vec::new();
        for value in values {
            if !expected.contains(&value) {
                expected.push(value);
            }
        }

        prop_assert_eq!(actual, expected);
    }
}

#[test]
fn filename_from_url_handles_common_and_degenerate_paths() {
    let cases = [
        ("", ""),
        ("file", "file"),
        ("https://example.test/file.txt", "file.txt"),
        ("https://example.test/path/", ""),
        ("/leading/path", "path"),
        ("path//file", "file"),
        ("path/file?raw=1", "file?raw=1"),
        ("path\\file", "path\\file"),
    ];

    for (url, expected) in cases {
        assert_eq!(filename_from_url(url), expected, "url: {url:?}");
    }
}

#[test]
fn default_config_paths_are_derived_without_environment_mutation() {
    let home = Path::new("/tmp/omnirepo-test-home");

    assert_eq!(
        default_config_paths(home),
        [
            PathBuf::from("/tmp/omnirepo-test-home/.omnirepo.yaml"),
            PathBuf::from("/tmp/omnirepo-test-home/.omnirepo/.omnirepo.yaml"),
        ]
    );
}

#[test]
fn load_config_accepts_a_file_or_directory() {
    let root = tempdir().unwrap();
    let file = root.path().join("custom.yaml");
    fs::write(&file, valid_config_yaml("file")).unwrap();

    let from_file = load_config(&file).unwrap();
    assert_eq!(from_file.config.repositories[0].name, "file");

    let directory = root.path().join("config");
    fs::create_dir(&directory).unwrap();
    fs::write(
        directory.join(".omnirepo.yaml"),
        valid_config_yaml("directory"),
    )
    .unwrap();

    let from_directory = load_config(&directory).unwrap();
    assert_eq!(from_directory.config.repositories[0].name, "directory");
}

#[test]
fn load_config_reports_missing_and_malformed_inputs() {
    let root = tempdir().unwrap();
    let missing = root.path().join("missing.yaml");
    let missing_error = match load_config(&missing) {
        Ok(_) => panic!("missing config should fail to load"),
        Err(error) => error.to_string(),
    };
    assert!(missing_error.contains("Could not open config file"));

    let malformed = root.path().join("malformed.yaml");
    fs::write(&malformed, "repositories: [\ntemplates: []\n").unwrap();
    let malformed_error = match load_config(&malformed) {
        Ok(_) => panic!("malformed config should fail to load"),
        Err(error) => error.to_string(),
    };
    assert!(malformed_error.contains("Error parsing YAML file"));

    let empty_directory = root.path().join("empty");
    fs::create_dir(&empty_directory).unwrap();
    assert!(load_config(&empty_directory).is_err());
}

#[test]
fn default_loader_prefers_home_file_then_falls_back_to_nested_file() {
    let root = tempdir().unwrap();
    let nested_directory = root.path().join(".omnirepo");
    fs::create_dir(&nested_directory).unwrap();
    fs::write(
        nested_directory.join(".omnirepo.yaml"),
        valid_config_yaml("nested"),
    )
    .unwrap();

    let nested = load_config_default_from_home(root.path()).unwrap();
    assert_eq!(nested.config.repositories[0].name, "nested");

    fs::write(
        root.path().join(".omnirepo.yaml"),
        valid_config_yaml("home"),
    )
    .unwrap();
    let home = load_config_default_from_home(root.path()).unwrap();
    assert_eq!(home.config.repositories[0].name, "home");
}

#[test]
fn default_loader_returns_not_found_without_either_candidate() {
    let root = tempdir().unwrap();

    let error = match load_config_default_from_home(root.path()) {
        Ok(_) => panic!("default loader should fail without a config"),
        Err(error) => error.to_string(),
    };

    assert!(error.contains("Default config file not found"));
}

#[test]
fn join_relative_accepts_normal_nested_and_dot_paths() {
    let base = Path::new("/tmp/base");

    assert_eq!(
        join_relative(base, "file.txt", "artifact").unwrap(),
        PathBuf::from("/tmp/base/file.txt")
    );
    assert_eq!(
        join_relative(base, "nested/file.txt", "artifact").unwrap(),
        PathBuf::from("/tmp/base/nested/file.txt")
    );
    assert_eq!(
        join_relative(base, "./nested/file.txt", "artifact").unwrap(),
        PathBuf::from("/tmp/base/./nested/file.txt")
    );
}

#[test]
fn join_relative_rejects_absolute_paths_with_context() {
    let error = join_relative(Path::new("/tmp/base"), "/etc/passwd", "artifact path")
        .expect_err("absolute paths must be rejected");

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("artifact path"));
    assert!(error.to_string().contains("absolute path"));
}

#[test]
fn join_relative_rejects_parent_traversal_with_context() {
    let error = join_relative(
        Path::new("/tmp/base"),
        "nested/../../file.txt",
        "artifact path",
    )
    .expect_err("parent traversal must be rejected");

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("artifact path"));
    assert!(error.to_string().contains("parent-directory traversal"));
}

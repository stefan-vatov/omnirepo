//! Focused proof for the strict machine-config YAML subset parser.

#![allow(dead_code, unused_imports)]

use super::yaml_subset::{YValue, parse_yaml_subset};

fn map_value(text: &str) -> YValue {
    parse_yaml_subset(text).expect("valid subset document")
}

fn string(value: &YValue) -> String {
    value.as_str().expect("string").to_owned()
}

#[test]
fn flat_mapping_with_scalars_parses() {
    let value = map_value("version: 1\ncache_root: /var/cache/omnirepo\nname: plain\n");
    let YValue::Map(entries) = &value else {
        panic!("expected map");
    };
    assert_eq!(entries.len(), 3);
    assert_eq!(value.get("version").and_then(YValue::as_u64), Some(1));
    assert_eq!(
        string(value.get("cache_root").expect("cache_root")),
        "/var/cache/omnirepo"
    );
}

#[test]
fn quoted_scalars_and_inline_lists_parse() {
    let value = map_value(
        "title: \"quoted text\"\nsingle: 'also quoted'\ntags: [prod, staging]\nempty: []\n",
    );
    assert_eq!(string(value.get("title").expect("title")), "quoted text");
    assert_eq!(string(value.get("single").expect("single")), "also quoted");
    let tags = value.get("tags").expect("tags").as_list().expect("list");
    assert_eq!(tags.len(), 2);
    assert_eq!(string(&tags[0]), "prod");
    assert_eq!(string(&tags[1]), "staging");
    assert!(
        value
            .get("empty")
            .expect("empty")
            .as_list()
            .expect("list")
            .is_empty()
    );
}

#[test]
fn nested_mappings_and_sequences_parse() {
    let value = map_value(
        "concurrency:\n  max_repositories: 8\n  max_child_work: 16\nrepair:\n  priority:\n    - codex\n    - pi\n  max_attempts: 3\n",
    );
    let concurrency = value.get("concurrency").expect("concurrency");
    assert_eq!(
        concurrency.get("max_repositories").and_then(YValue::as_u64),
        Some(8)
    );
    assert_eq!(
        concurrency.get("max_child_work").and_then(YValue::as_u64),
        Some(16)
    );
    let repair = value.get("repair").expect("repair");
    let priority = repair
        .get("priority")
        .expect("priority")
        .as_list()
        .expect("list");
    assert_eq!(priority.len(), 2);
    assert_eq!(string(&priority[0]), "codex");
    assert_eq!(repair.get("max_attempts").and_then(YValue::as_u64), Some(3));
}

#[test]
fn comments_and_blank_lines_are_ignored() {
    let value = map_value(
        "# leading comment\nversion: 1 # trailing comment\n\nname: \"a#b\" # hash inside quotes\n",
    );
    assert_eq!(value.get("version").and_then(YValue::as_u64), Some(1));
    assert_eq!(string(value.get("name").expect("name")), "a#b");
}

#[test]
fn tabs_in_indentation_fail_closed() {
    let error = parse_yaml_subset("version: 1\n\tname: tabbed\n").expect_err("tabs rejected");
    assert!(error.reason.contains("tabs"), "{error:?}");
}

#[test]
fn duplicate_keys_fail_closed() {
    let error = parse_yaml_subset("version: 1\nversion: 2\n").expect_err("duplicate keys rejected");
    assert!(error.reason.contains("duplicate"), "{error:?}");
}

#[test]
fn unterminated_quotes_and_inline_lists_fail_closed() {
    assert!(parse_yaml_subset("name: \"unterminated\n").is_err());
    assert!(parse_yaml_subset("tags: [a, b\n").is_err());
}

#[test]
fn flow_mappings_and_escapes_fail_closed() {
    assert!(parse_yaml_subset("nested: {a: 1}\n").is_err());
    assert!(parse_yaml_subset("name: \"has\\nescape\"\n").is_err());
}

#[test]
fn top_level_must_be_a_mapping() {
    assert!(parse_yaml_subset("- item\n").is_err());
    assert!(parse_yaml_subset("just a scalar\n").is_err());
}

#[test]
fn inconsistent_indentation_fails_closed() {
    let error = parse_yaml_subset("concurrency:\n  max_repositories: 8\n    max_child_work: 16\n")
        .expect_err("inconsistent indentation rejected");
    assert!(error.reason.contains("indentation"), "{error:?}");
}

use std::fs;
use std::path::PathBuf;

use yaml_serde::{Mapping, Value};

const UNSAFE_UNTAGGED_MAIN_PUSH: &str =
    include_str!("fixtures/release_quarantine/untagged-main-push.yml");

const MUTABLE_WRITE_AUTHORITY_FIXTURE: &str = r#"
name: mutable reusable publication
on:
  push:
    tags:
      - v*
jobs:
  publish:
    permissions:
      contents: write
    uses: acme/release/.github/workflows/publish.yml@main
"#;

const PERMISSION_ESCALATION_FIXTURE: &str = r#"
name: permission escalation
on:
  push:
    branches:
      - main
permissions:
  contents: read
jobs:
  publish:
    permissions:
      contents: write
    steps:
      - run: gh release create v0.0.0
"#;

const TAG_BUILD_SHA_DIVERGENCE_FIXTURE: &str = r#"
name: divergent tag release
on:
  push:
    tags:
      - v*
permissions:
  contents: write
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: cargo build --release
  publish:
    needs: build
    permissions:
      contents: write
    steps:
      - run: git fetch --tags
      - run: |
          latest_tag="$(git tag --list 'v*' | sort -V | tail -n 1)"
          echo "tag=$latest_tag" >> "$GITHUB_OUTPUT"
      - uses: softprops/action-gh-release@v3
        with:
          tag_name: ${{ steps.latest_tag.outputs.tag }}
"#;

const SAFE_TAGGED_SHA_BOUND_FIXTURE: &str = r#"
name: qualified tagged release
on:
  push:
    tags:
      - v*
permissions:
  contents: write
jobs:
  publish:
    uses: acme/release/.github/workflows/publish.yml@0123456789abcdef0123456789abcdef01234567
    with:
      source_sha: ${{ github.sha }}
"#;

const UNSAFE_COG_HOOKS: &str = r#"
post_bump_hooks = [
    "git push",
    "git tag {{version}}"
]
"#;

const PUBLICATION_MARKERS: &[&str] = &[
    "actions/create-release",
    "actions/upload-release-asset",
    "bump-and-tag.yml",
    "cargo publish",
    "cog bump",
    "git push",
    "git tag",
    "gh release",
    "softprops/action-gh-release",
];

#[derive(Debug, PartialEq, Eq)]
struct Violation {
    path: String,
    reason: String,
}

struct ParsedWorkflow {
    path: String,
    document: Value,
}

impl ParsedWorkflow {
    fn parse(path: &str, source: &str) -> Self {
        let document = yaml_serde::from_str::<Value>(source)
            .unwrap_or_else(|error| panic!("workflow={path}; reason=invalid YAML: {error}"));
        assert!(
            document.as_mapping().is_some(),
            "workflow={path}; reason=workflow root must be a mapping"
        );
        Self {
            path: path.to_owned(),
            document,
        }
    }

    fn root(&self) -> &Mapping {
        self.document
            .as_mapping()
            .expect("parsed workflow root was checked during construction")
    }
}

fn workflows_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".github/workflows")
}

fn workflow_sources() -> Vec<(PathBuf, String)> {
    let mut paths = fs::read_dir(workflows_dir())
        .expect("workflow directory must exist")
        .map(|entry| {
            entry
                .expect("workflow directory entry must be readable")
                .path()
        })
        .filter(|path| {
            matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("yml" | "yaml")
            )
        })
        .collect::<Vec<_>>();
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            let source = fs::read_to_string(&path).expect("workflow must be readable");
            (path, source)
        })
        .collect()
}

fn mapping_value<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a Value> {
    mapping.iter().find_map(|(candidate, value)| {
        if candidate.as_str() == Some(key) || (key == "on" && candidate.as_bool() == Some(true)) {
            Some(value)
        } else {
            None
        }
    })
}

fn scalar_strings(value: &Value, output: &mut Vec<String>) {
    if let Some(string) = value.as_str() {
        output.push(string.to_owned());
        return;
    }
    if let Some(sequence) = value.as_sequence() {
        for item in sequence {
            scalar_strings(item, output);
        }
        return;
    }
    if let Some(mapping) = value.as_mapping() {
        for (key, item) in mapping {
            scalar_strings(key, output);
            scalar_strings(item, output);
        }
    }
}

fn keyed_strings(value: &Value, key: &str, path: &str, output: &mut Vec<(String, String)>) {
    if let Some(mapping) = value.as_mapping() {
        for (candidate, item) in mapping {
            let candidate_text = candidate.as_str().unwrap_or("<non-string-key>");
            let child_path = format!("{path}.{candidate_text}");
            if candidate_text == key
                && let Some(string) = item.as_str()
            {
                output.push((child_path.clone(), string.to_owned()));
            }
            keyed_strings(item, key, &child_path, output);
        }
    } else if let Some(sequence) = value.as_sequence() {
        for (index, item) in sequence.iter().enumerate() {
            keyed_strings(item, key, &format!("{path}[{index}]"), output);
        }
    }
}

fn is_immutable_revision(reference: &str) -> bool {
    reference.rsplit_once('@').is_some_and(|(_, revision)| {
        revision.len() == 40
            && revision
                .chars()
                .all(|character| character.is_ascii_hexdigit())
    })
}

fn mutable_reusable_refs(value: &Value, path: &str) -> Vec<(String, String)> {
    let mut references = Vec::new();
    keyed_strings(value, "uses", path, &mut references);
    references
        .into_iter()
        .filter(|(_, reference)| {
            reference.contains("/.github/workflows/") && !is_immutable_revision(reference)
        })
        .collect()
}

fn branch_filter_values(value: &Value) -> Vec<String> {
    if let Some(string) = value.as_str() {
        return vec![string.to_owned()];
    }
    value
        .as_sequence()
        .map(|sequence| {
            sequence
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn includes_main_branch(value: &Value) -> bool {
    branch_filter_values(value)
        .iter()
        .any(|branch| branch == "main" || branch == "*" || branch == "**")
}

fn push_targets_main(push: &Value) -> bool {
    match push {
        Value::Null => true,
        Value::Mapping(mapping) => {
            if mapping_value(mapping, "branches-ignore").is_some_and(includes_main_branch) {
                return false;
            }
            if mapping_value(mapping, "tags").is_some()
                && mapping_value(mapping, "branches").is_none()
            {
                return false;
            }
            mapping_value(mapping, "branches").is_none_or(includes_main_branch)
        }
        Value::Sequence(_) | Value::String(_) => true,
        _ => false,
    }
}

fn triggers_main_push(root: &Mapping) -> bool {
    let Some(on) = mapping_value(root, "on") else {
        return false;
    };
    match on {
        Value::Mapping(mapping) => mapping_value(mapping, "push").is_some_and(push_targets_main),
        Value::Sequence(sequence) => sequence.iter().any(|event| event.as_str() == Some("push")),
        Value::String(event) => event == "push",
        _ => false,
    }
}

fn has_owner_selected_trigger(root: &Mapping) -> bool {
    let Some(on) = mapping_value(root, "on") else {
        return false;
    };
    let Value::Mapping(mapping) = on else {
        return false;
    };
    if mapping_value(mapping, "workflow_dispatch").is_some()
        || mapping_value(mapping, "release").is_some()
    {
        return true;
    }
    mapping_value(mapping, "push")
        .and_then(Value::as_mapping)
        .and_then(|push| mapping_value(push, "tags"))
        .is_some_and(|tags| !branch_filter_values(tags).is_empty())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContentsPermission {
    Unspecified,
    Read,
    Write,
    Other,
}

fn contents_permission(value: Option<&Value>) -> ContentsPermission {
    let Some(value) = value else {
        return ContentsPermission::Unspecified;
    };
    if let Some(permission) = value.as_str() {
        return match permission {
            "read-all" | "read" => ContentsPermission::Read,
            "write-all" | "write" => ContentsPermission::Write,
            _ => ContentsPermission::Other,
        };
    }
    value
        .as_mapping()
        .and_then(|mapping| mapping_value(mapping, "contents"))
        .map_or(ContentsPermission::Other, |contents| {
            contents_permission(Some(contents))
        })
}

fn effective_contents_permission(root: &Mapping, job: &Mapping) -> ContentsPermission {
    let job_permission = contents_permission(mapping_value(job, "permissions"));
    if job_permission != ContentsPermission::Unspecified {
        job_permission
    } else {
        contents_permission(mapping_value(root, "permissions"))
    }
}

fn publication_markers(value: &Value, path: &str) -> Vec<String> {
    let mut hits = Vec::new();
    if let Some(mapping) = value.as_mapping() {
        let uses = mapping_value(mapping, "uses").and_then(Value::as_str);
        if uses.is_some_and(|uses| uses.contains("actions/upload-artifact")) {
            let artifact_name = mapping_value(mapping, "with")
                .and_then(Value::as_mapping)
                .and_then(|with| mapping_value(with, "name"))
                .and_then(Value::as_str);
            if artifact_name != Some("coverage-reports") {
                hits.push(format!("{path}: non-coverage artifact upload"));
            }
        }
        for (key, item) in mapping {
            let key_text = key.as_str().unwrap_or("<non-string-key>");
            let child_path = format!("{path}.{key_text}");
            if key_text == "tag_name" {
                hits.push(format!("{child_path}: tag_name publication selector"));
            }
            if let Some(string) = item.as_str() {
                for marker in PUBLICATION_MARKERS {
                    if string.contains(marker) {
                        hits.push(format!("{child_path}: contains {marker}"));
                    }
                }
            }
            hits.extend(publication_markers(item, &child_path));
        }
    } else if let Some(sequence) = value.as_sequence() {
        for (index, item) in sequence.iter().enumerate() {
            hits.extend(publication_markers(item, &format!("{path}[{index}]")));
        }
    }
    hits
}

fn has_tag_build_sha_divergence(workflow: &ParsedWorkflow) -> bool {
    let mut texts = Vec::new();
    scalar_strings(&workflow.document, &mut texts);
    let has_build = texts
        .iter()
        .any(|text| text.contains("cargo build") || text.contains("cargo build --release"));
    let has_independent_tag = texts.iter().any(|text| {
        text.contains("git tag --list")
            || text.contains("refs/tags")
            || text.contains("latest_tag")
            || text == "tag_name"
    });
    let has_sha_binding = texts.iter().any(|text| {
        text.contains("github.sha")
            || text.contains("GITHUB_SHA")
            || text.contains("git rev-parse HEAD")
            || text.contains("commit_sha")
    });
    has_build && has_independent_tag && !has_sha_binding
}

fn evaluate_workflow(workflow: &ParsedWorkflow) -> Vec<Violation> {
    let root = workflow.root();
    let main_push = triggers_main_push(root);
    let owner_selected = has_owner_selected_trigger(root);
    let root_contents = contents_permission(mapping_value(root, "permissions"));
    let mut violations = Vec::new();
    let Some(jobs) = mapping_value(root, "jobs").and_then(Value::as_mapping) else {
        return violations;
    };

    for (job_name, job) in jobs {
        let job_name = job_name.as_str().unwrap_or("<non-string-job>");
        let job_path = format!("{}::jobs.{job_name}", workflow.path);
        let Some(job_mapping) = job.as_mapping() else {
            violations.push(Violation {
                path: job_path,
                reason: "job definition is not a mapping".to_owned(),
            });
            continue;
        };

        let job_contents = contents_permission(mapping_value(job_mapping, "permissions"));
        let effective_contents = effective_contents_permission(root, job_mapping);
        if root_contents == ContentsPermission::Read && job_contents == ContentsPermission::Write {
            violations.push(Violation {
                path: job_path.clone(),
                reason: "job contents write escalates a workflow-level contents read".to_owned(),
            });
        }

        let markers = publication_markers(job, &job_path);
        let mutable_refs = mutable_reusable_refs(job, &job_path);
        let has_mutable_refs = !mutable_refs.is_empty();
        if effective_contents == ContentsPermission::Write && (!owner_selected || main_push) {
            violations.push(Violation {
                path: job_path.clone(),
                reason: "write authority is reachable from an untagged or main push instead of an owner-selected trigger".to_owned(),
            });
        }
        for (reference_path, reference) in mutable_refs {
            if effective_contents == ContentsPermission::Write {
                violations.push(Violation {
                    path: reference_path,
                    reason: format!(
                        "mutable reusable workflow ref {reference} has contents write authority"
                    ),
                });
            }
        }
        if main_push
            && (!markers.is_empty()
                || (effective_contents == ContentsPermission::Write && has_mutable_refs))
        {
            violations.push(Violation {
                path: job_path.clone(),
                reason: format!(
                    "main push can reach publication effect: {}",
                    if markers.is_empty() {
                        "mutable reusable workflow with write authority".to_owned()
                    } else {
                        markers.join("; ")
                    }
                ),
            });
        }
        if effective_contents == ContentsPermission::Write
            && !markers.is_empty()
            && has_tag_build_sha_divergence(workflow)
        {
            violations.push(Violation {
                path: job_path,
                reason: "publication tag is selected independently from the built commit SHA"
                    .to_owned(),
            });
        }
    }
    violations
}

fn parsed_workflow_violations(path: &str, source: &str) -> Vec<Violation> {
    evaluate_workflow(&ParsedWorkflow::parse(path, source))
}

fn current_workflow_violations() -> Vec<Violation> {
    workflow_sources()
        .into_iter()
        .flat_map(|(path, source)| {
            let path = path.display().to_string();
            parsed_workflow_violations(&path, &source)
        })
        .collect()
}

fn parse_post_bump_hooks(source: &str) -> Vec<String> {
    let mut collecting = false;
    let mut hooks = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if !collecting && trimmed.starts_with("post_bump_hooks") {
            collecting = true;
        }
        if !collecting {
            continue;
        }
        let quoted = trimmed
            .split('"')
            .enumerate()
            .filter_map(|(index, part)| (index % 2 == 1).then_some(part.to_owned()));
        hooks.extend(quoted);
        if trimmed.contains(']') {
            break;
        }
    }
    hooks
}

fn assert_violation(violations: &[Violation], reason_fragment: &str, fixture: &str) {
    let matching = violations
        .iter()
        .find(|violation| violation.reason.contains(reason_fragment));
    let Some(matching) = matching else {
        panic!(
            "fixture={fixture}; expected reason containing {reason_fragment:?}; observed violations={violations:?}"
        );
    };
    assert!(
        !matching.path.is_empty(),
        "fixture={fixture}; violation path is empty"
    );
    assert!(
        !matching.reason.is_empty(),
        "fixture={fixture}; violation reason is empty"
    );
}

#[test]
fn obsolete_main_push_publication_workflows_are_removed() {
    assert!(!workflows_dir().join("bump.yml").exists());
    assert!(!workflows_dir().join("create-release.yml").exists());
}

#[test]
fn current_workflows_parse_to_a_green_quarantine_policy() {
    let violations = current_workflow_violations();
    assert!(
        violations.is_empty(),
        "current workflow quarantine violations with actionable path and reason: {violations:?}"
    );
}

#[test]
fn parsed_untagged_main_push_fixture_is_rejected() {
    let violations = parsed_workflow_violations(
        "fixtures/release_quarantine/untagged-main-push.yml",
        UNSAFE_UNTAGGED_MAIN_PUSH,
    );
    assert_violation(
        &violations,
        "main push can reach publication",
        "fixtures/release_quarantine/untagged-main-push.yml",
    );
}

#[test]
fn parsed_mutable_reusable_write_authority_is_rejected() {
    let violations = parsed_workflow_violations(
        "fixture://mutable-write-authority.yml",
        MUTABLE_WRITE_AUTHORITY_FIXTURE,
    );
    assert_violation(
        &violations,
        "mutable reusable workflow ref",
        "fixture://mutable-write-authority.yml",
    );
}

#[test]
fn parsed_permission_escalation_is_rejected() {
    let violations = parsed_workflow_violations(
        "fixture://permission-escalation.yml",
        PERMISSION_ESCALATION_FIXTURE,
    );
    assert_violation(
        &violations,
        "escalates a workflow-level contents read",
        "fixture://permission-escalation.yml",
    );
}

#[test]
fn parsed_tag_build_sha_divergence_is_rejected() {
    let violations = parsed_workflow_violations(
        "fixture://tag-build-sha-divergence.yml",
        TAG_BUILD_SHA_DIVERGENCE_FIXTURE,
    );
    assert_violation(
        &violations,
        "publication tag is selected independently",
        "fixture://tag-build-sha-divergence.yml",
    );
}

#[test]
fn parsed_owner_selected_sha_bound_fixture_is_allowed() {
    let violations = parsed_workflow_violations(
        "fixture://tagged-sha-bound.yml",
        SAFE_TAGGED_SHA_BOUND_FIXTURE,
    );
    assert!(
        violations.is_empty(),
        "fixture=fixture://tagged-sha-bound.yml; expected green policy, observed violations={violations:?}"
    );
}

#[test]
fn cog_post_bump_hooks_are_structurally_empty() {
    let cog = include_str!("../cog.toml");
    let hooks = parse_post_bump_hooks(cog);
    assert!(
        hooks.is_empty(),
        "path=cog.toml; reason=post-bump publication hooks remain: {hooks:?}"
    );
}

#[test]
fn parsed_cog_post_bump_publication_hooks_are_rejected() {
    let hooks = parse_post_bump_hooks(UNSAFE_COG_HOOKS);
    assert_eq!(hooks, vec!["git push", "git tag {{version}}"]);
    assert!(
        hooks
            .iter()
            .any(|hook| hook == "git push" || hook == "git tag {{version}}"),
        "fixture=fixture://unsafe-cog.toml; reason=publication hook was not identified: {hooks:?}"
    );
}

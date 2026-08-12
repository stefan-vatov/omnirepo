use super::*;
use proptest::prelude::*;

fn scalar() -> impl Strategy<Value = String> {
    // Keep generated values readable while still exercising YAML quoting
    // and escaping through punctuation that is common in config values.
    "[A-Za-z0-9_./:-]{1,16}".prop_map(|value| value)
}

fn tags() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(scalar(), 0..5)
}

fn repository() -> impl Strategy<Value = Repository> {
    (scalar(), scalar(), tags(), scalar()).prop_map(|(name, url, tags, dest)| Repository {
        name,
        url,
        tags,
        dest,
    })
}

fn included_file() -> impl Strategy<Value = IncludedFile> {
    (scalar(), scalar(), scalar()).prop_map(|(file_name, id, dest)| IncludedFile {
        file_name,
        id,
        dest,
    })
}

fn template() -> impl Strategy<Value = Template> {
    (
        scalar(),
        scalar(),
        scalar(),
        prop_oneof![Just(TemplateType::File), Just(TemplateType::Dir)],
        prop::option::of(scalar()),
        tags(),
        prop::option::of(prop::collection::vec(included_file(), 0..5)),
    )
        .prop_map(
            |(name, id, url, kind, dest, tags, included_files)| Template {
                name,
                id,
                url,
                kind,
                dest,
                tags,
                included_files,
            },
        )
}

fn config() -> impl Strategy<Value = Config> {
    (
        prop::collection::vec(repository(), 0..5),
        prop::collection::vec(template(), 0..5),
    )
        .prop_map(|(repositories, templates)| Config {
            repositories,
            templates,
        })
}

#[test]
fn config_yaml_roundtrip_preserves_all_values() {
    let original = Config {
        repositories: vec![Repository {
            name: "dotfiles".into(),
            url: "https://example.test/dotfiles.git".into(),
            tags: vec!["config".into(), "default".into()],
            dest: "dotfiles".into(),
        }],
        templates: vec![Template {
            name: "workflow".into(),
            id: "workflow-v1".into(),
            url: "https://example.test/templates/workflows".into(),
            kind: TemplateType::Dir,
            dest: None,
            tags: vec!["ci".into()],
            included_files: Some(vec![IncludedFile {
                file_name: "ci.yml".into(),
                id: "ci-v1".into(),
                dest: ".github/workflows".into(),
            }]),
        }],
    };

    let yaml = yaml_serde::to_string(&original).expect("config should serialize");
    let parsed: Config = yaml_serde::from_str(&yaml).expect("serialized config should parse");

    assert_eq!(parsed, original);
}

#[test]
fn wrapper_and_repo_config_values_roundtrip() {
    let repositories = Repositories {
        repositories: vec![Repository {
            name: "repo".into(),
            url: "url".into(),
            tags: vec!["tag".into()],
            dest: "dest".into(),
        }],
    };
    let templates = Templates {
        templates: vec![Template {
            name: "file".into(),
            id: "file-id".into(),
            url: "url/file".into(),
            kind: TemplateType::File,
            dest: Some(".".into()),
            tags: vec!["tag".into()],
            included_files: None,
        }],
    };
    let repo_config = RepoConfig::new(vec!["one".into(), "two".into()]);

    let repositories_yaml = yaml_serde::to_string(&repositories).unwrap();
    let templates_yaml = yaml_serde::to_string(&templates).unwrap();
    let repo_config_yaml = yaml_serde::to_string(&repo_config).unwrap();

    assert_eq!(
        yaml_serde::from_str::<Repositories>(&repositories_yaml).unwrap(),
        repositories
    );
    assert_eq!(
        yaml_serde::from_str::<Templates>(&templates_yaml).unwrap(),
        templates
    );
    assert_eq!(
        yaml_serde::from_str::<RepoConfig>(&repo_config_yaml).unwrap(),
        repo_config
    );
}

#[test]
fn optional_template_fields_may_be_omitted() {
    let template: Template = yaml_serde::from_str(
        "name: file\nid: file-id\nurl: https://example.test/file\nkind: File\ntags: []\n",
    )
    .expect("dest and included_files are optional");

    assert_eq!(template.dest, None);
    assert_eq!(template.included_files, None);
}

#[test]
fn required_template_and_included_file_ids_reject_missing_fields() {
    let missing_template_id = "name: file\nurl: https://example.test/file\nkind: File\ntags: []\n";
    let missing_included_file_id = "file_name: file\ndest: .config\n";

    assert!(yaml_serde::from_str::<Template>(missing_template_id).is_err());
    assert!(yaml_serde::from_str::<IncludedFile>(missing_included_file_id).is_err());
}

#[test]
fn malformed_values_and_missing_top_level_fields_reject() {
    let invalid_kind =
        "name: file\nid: file-id\nurl: https://example.test/file\nkind: Unknown\ntags: []\n";
    let missing_repositories = "templates: []\n";
    let missing_templates = "repositories: []\n";
    let malformed_yaml = "repositories: [\ntemplates: []\n";

    assert!(yaml_serde::from_str::<Template>(invalid_kind).is_err());
    assert!(yaml_serde::from_str::<Config>(missing_repositories).is_err());
    assert!(yaml_serde::from_str::<Config>(missing_templates).is_err());
    assert!(yaml_serde::from_str::<Config>(malformed_yaml).is_err());
}

proptest! {
    #[test]
    fn arbitrary_configs_roundtrip_through_yaml(original in config()) {
        let yaml = yaml_serde::to_string(&original).expect("generated config should serialize");
        let parsed: Config = yaml_serde::from_str(&yaml).expect("serialized config should parse");

        prop_assert_eq!(parsed, original);
    }
}

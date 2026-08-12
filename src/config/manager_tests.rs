use super::*;
use crate::config::parser::{IncludedFile, Repository, Template};
use proptest::prelude::*;

fn repository(name: &str, url: &str, tags: &[&str], dest: &str) -> Repository {
    Repository {
        name: name.into(),
        url: url.into(),
        tags: tags.iter().map(|tag| (*tag).into()).collect(),
        dest: dest.into(),
    }
}

fn file_template(name: &str, id: &str, url: &str, dest: Option<&str>, tags: &[&str]) -> Template {
    Template {
        name: name.into(),
        id: id.into(),
        url: url.into(),
        kind: TemplateType::File,
        dest: dest.map(str::to_owned),
        tags: tags.iter().map(|tag| (*tag).into()).collect(),
        included_files: None,
    }
}

fn dir_template(
    name: &str,
    id: &str,
    url: &str,
    included_files: Option<Vec<IncludedFile>>,
    tags: &[&str],
) -> Template {
    Template {
        name: name.into(),
        id: id.into(),
        url: url.into(),
        kind: TemplateType::Dir,
        dest: None,
        tags: tags.iter().map(|tag| (*tag).into()).collect(),
        included_files,
    }
}

fn included_file(file_name: &str, id: &str, dest: &str) -> IncludedFile {
    IncludedFile {
        file_name: file_name.into(),
        id: id.into(),
        dest: dest.into(),
    }
}

fn manager() -> GlobalConfigManager {
    GlobalConfigManager::new(Config {
        repositories: vec![
            repository(
                "one",
                "https://example.test/one.git",
                &["all", "work"],
                "one",
            ),
            repository("two", "https://example.test/two.git", &["all"], "two"),
            repository(
                "three",
                "https://example.test/three.git",
                &["personal"],
                "three",
            ),
            repository("untagged", "https://example.test/none.git", &[], "none"),
        ],
        templates: vec![
            file_template(
                "readme",
                "readme-id",
                "https://example.test/readme",
                Some("docs"),
                &["all", "docs"],
            ),
            file_template(
                "without-dest",
                "without-dest-id",
                "https://example.test/no-dest",
                None,
                &["all"],
            ),
            dir_template(
                "workflows",
                "workflows-id",
                "https://example.test/workflows",
                Some(vec![
                    included_file("ci.yml", "ci-id", ".github/workflows"),
                    included_file("release.yml", "release-id", ".github/workflows"),
                ]),
                &["all", "ci"],
            ),
            dir_template(
                "without-files",
                "without-files-id",
                "https://example.test/empty",
                None,
                &["all"],
            ),
        ],
    })
}

#[test]
fn repository_tag_queries_preserve_config_order_and_ignore_non_matches() {
    let manager = manager();

    assert_eq!(
        manager.get_url_by_tag("all"),
        vec![
            "https://example.test/one.git",
            "https://example.test/two.git",
        ]
    );
    assert_eq!(manager.get_dest_by_tag("all"), vec!["one", "two"]);
    assert_eq!(manager.get_url_by_tag("missing"), Vec::<String>::new());
    assert_eq!(manager.get_dest_by_tag("personal"), vec!["three"]);
}

#[test]
fn template_queries_cover_file_and_directory_shapes() {
    let manager = manager();

    assert_eq!(
        manager.template_and_dest("all"),
        vec![
            ("https://example.test/readme".into(), "docs".into()),
            (
                "https://example.test/workflows/ci.yml".into(),
                ".github/workflows".into(),
            ),
            (
                "https://example.test/workflows/release.yml".into(),
                ".github/workflows".into(),
            ),
        ]
    );
    assert_eq!(
        manager.template_and_dest("docs"),
        vec![("https://example.test/readme".into(), "docs".into())]
    );
    assert!(manager.template_and_dest("missing").is_empty());
}

#[test]
fn repo_config_manager_exposes_dirs_without_reordering_or_copying() {
    let manager = RepoConfigManager::new(RepoConfig::new(vec!["first".into(), "second".into()]));

    assert_eq!(manager.get_dirs(), ["first", "second"]);
}

proptest! {
    #[test]
    fn repository_tag_queries_match_a_direct_filter(
        repositories in prop::collection::vec(
            ("[a-z]{1,8}", "[a-z:/._-]{1,16}", prop::collection::vec("[a-z]{1,5}", 0..4), "[a-z]{1,8}"),
            0..10,
        ),
        query in "[a-z]{1,5}",
    ) {
        let repositories: Vec<Repository> = repositories
            .into_iter()
            .map(|(name, url, tags, dest)| Repository { name, url, tags, dest })
            .collect();
        let expected_urls: Vec<String> = repositories
            .iter()
            .filter(|repo| repo.tags.iter().any(|tag| tag == &query))
            .map(|repo| repo.url.clone())
            .collect();
        let expected_dests: Vec<String> = repositories
            .iter()
            .filter(|repo| repo.tags.iter().any(|tag| tag == &query))
            .map(|repo| repo.dest.clone())
            .collect();
        let manager = GlobalConfigManager::new(Config { repositories, templates: vec![] });

        prop_assert_eq!(manager.get_url_by_tag(&query), expected_urls);
        prop_assert_eq!(manager.get_dest_by_tag(&query), expected_dests);
    }
}

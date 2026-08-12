use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    thread,
    time::{Duration, Instant},
};

use reqwest::blocking;

use super::*;
use crate::config::{
    manager::GlobalConfigManager,
    parser::{Config, IncludedFile},
};

const LOCAL_HTTP_TIMEOUT: Duration = Duration::from_secs(2);

fn manager_with_templates(templates: Vec<Template>) -> GlobalConfigManager {
    GlobalConfigManager::new(Config {
        repositories: Vec::new(),
        templates,
    })
}

fn write_repo_config(destination: &Path, dirs: &[&str]) {
    let yaml = dirs
        .iter()
        .map(|dir| format!("  - {dir}"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(destination.join(".omni.yaml"), format!("dirs:\n{yaml}\n")).unwrap();
    for dir in dirs {
        fs::create_dir_all(destination.join(dir)).unwrap();
    }
}

fn write_repo_config_without_roots(destination: &Path, dirs: &[&str]) {
    let yaml = dirs
        .iter()
        .map(|dir| format!("  - {dir}"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(destination.join(".omni.yaml"), format!("dirs:\n{yaml}\n")).unwrap();
}

fn file_template(id: &str, url: &str) -> Template {
    Template {
        name: "file template".to_string(),
        id: id.to_string(),
        url: url.to_string(),
        kind: TemplateType::File,
        dest: None,
        tags: Vec::new(),
        included_files: None,
    }
}

fn directory_template(url: &str, included_files: Vec<IncludedFile>) -> Template {
    Template {
        name: "directory template".to_string(),
        id: "directory-id".to_string(),
        url: url.to_string(),
        kind: TemplateType::Dir,
        dest: None,
        tags: Vec::new(),
        included_files: Some(included_files),
    }
}

fn local_http_client() -> blocking::Client {
    blocking::Client::builder()
        .no_proxy()
        .timeout(LOCAL_HTTP_TIMEOUT)
        .build()
        .unwrap()
}

fn local_http_server(status: &str, body: &str) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let status = status.to_string();
    let body = body.to_string();

    let server = thread::spawn(move || {
        let deadline = Instant::now() + LOCAL_HTTP_TIMEOUT;
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_read_timeout(Some(LOCAL_HTTP_TIMEOUT)).unwrap();
                    stream.set_write_timeout(Some(LOCAL_HTTP_TIMEOUT)).unwrap();

                    let mut request = [0_u8; 4096];
                    let _ = stream.read(&mut request);
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        break;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });

    (format!("http://{address}"), server)
}

#[test]
fn source_selection_identifies_each_supported_selector() {
    assert_eq!(
        select_source(Some("https://example.test/file".to_string()), None, None),
        Ok(SourceSelector::Url("https://example.test/file".to_string()))
    );
    assert_eq!(
        select_source(None, Some("template-id".to_string()), None),
        Ok(SourceSelector::TemplateId("template-id".to_string()))
    );
    assert_eq!(
        select_source(None, None, Some("source.txt".to_string())),
        Ok(SourceSelector::LocalFile("source.txt".to_string()))
    );
}

#[test]
fn source_selection_rejects_missing_source() {
    let error = select_source(None, None, None).unwrap_err();

    assert!(error.contains("exactly one"));
}

#[test]
fn source_selection_rejects_conflicting_sources() {
    let error = select_source(
        Some("https://example.test/file".to_string()),
        Some("template-id".to_string()),
        None,
    )
    .unwrap_err();

    assert!(error.contains("Conflicting template sources"));
}

#[test]
fn file_template_id_resolves_to_its_url() {
    let templates = vec![file_template(
        "file-id",
        "https://example.test/template.txt",
    )];

    assert_eq!(
        find_url_by_id(&templates, "file-id"),
        Some("https://example.test/template.txt".to_string())
    );
}

#[test]
fn included_file_template_id_resolves_to_file_url() {
    let templates = vec![directory_template(
        "https://example.test/templates/",
        vec![IncludedFile {
            file_name: "/nested/template.txt".to_string(),
            id: "included-id".to_string(),
            dest: "nested/template.txt".to_string(),
        }],
    )];

    assert_eq!(
        find_url_by_id(&templates, "included-id"),
        Some("https://example.test/templates/nested/template.txt".to_string())
    );
}

#[test]
fn unknown_template_id_does_not_resolve() {
    let templates = vec![file_template(
        "file-id",
        "https://example.test/template.txt",
    )];

    assert_eq!(find_url_by_id(&templates, "unknown-id"), None);
}

#[test]
fn fetch_template_returns_body_for_successful_response() {
    let (url, server) = local_http_server("200 OK", "template contents");

    let result = fetch_template_with_client(&local_http_client(), &url);
    server.join().unwrap();

    assert_eq!(result.unwrap(), "template contents");
}

#[test]
fn fetch_template_returns_error_for_unsuccessful_status() {
    let (url, server) = local_http_server("404 Not Found", "not found");

    let error = fetch_template_with_client(&local_http_client(), &url)
        .unwrap_err()
        .to_string();
    server.join().unwrap();

    assert!(error.contains("404"));
}

#[test]
fn sync_file_updates_repositories_from_http_source() {
    let destination = tempfile::tempdir().unwrap();
    write_repo_config(destination.path(), &["repo"]);
    let (url, server) = local_http_server("200 OK", "http contents");

    sync_file_with_client(
        manager_with_templates(Vec::new()),
        "settings.txt".to_string(),
        Some(url),
        None,
        Some(destination.path().display().to_string()),
        None,
        &local_http_client(),
    )
    .unwrap();
    server.join().unwrap();

    assert_eq!(
        fs::read_to_string(destination.path().join("repo/settings.txt")).unwrap(),
        "http contents"
    );
}

#[test]
fn sync_file_updates_repositories_from_template_id() {
    let destination = tempfile::tempdir().unwrap();
    write_repo_config(destination.path(), &["repo"]);
    let (url, server) = local_http_server("200 OK", "template-id contents");

    sync_file_with_client(
        manager_with_templates(vec![file_template("template-id", &url)]),
        "settings.txt".to_string(),
        None,
        Some("template-id".to_string()),
        Some(destination.path().display().to_string()),
        None,
        &local_http_client(),
    )
    .unwrap();
    server.join().unwrap();

    assert_eq!(
        fs::read_to_string(destination.path().join("repo/settings.txt")).unwrap(),
        "template-id contents"
    );
}

#[test]
fn sync_file_reads_local_source_and_updates_all_repositories() {
    let destination = tempfile::tempdir().unwrap();
    write_repo_config(destination.path(), &["repo-a", "repo-b"]);
    fs::write(destination.path().join("source.txt"), "local contents").unwrap();

    sync_file(
        manager_with_templates(Vec::new()),
        "settings.txt".to_string(),
        None,
        None,
        Some(destination.path().display().to_string()),
        Some("source.txt".to_string()),
    )
    .unwrap();

    assert_eq!(
        fs::read_to_string(destination.path().join("repo-a/settings.txt")).unwrap(),
        "local contents"
    );
    assert_eq!(
        fs::read_to_string(destination.path().join("repo-b/settings.txt")).unwrap(),
        "local contents"
    );
}

#[test]
fn sync_file_returns_error_when_local_source_is_missing() {
    let destination = tempfile::tempdir().unwrap();

    let error = sync_file(
        manager_with_templates(Vec::new()),
        "settings.txt".to_string(),
        None,
        None,
        Some(destination.path().display().to_string()),
        Some("missing.txt".to_string()),
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("Could not read local source file"));
}

#[test]
fn sync_file_rejects_traversal_in_local_source() {
    let destination = tempfile::tempdir().unwrap();

    let error = sync_file(
        manager_with_templates(Vec::new()),
        "settings.txt".to_string(),
        None,
        None,
        Some(destination.path().display().to_string()),
        Some("../outside-source.txt".to_string()),
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("parent-directory traversal"));
}

#[test]
fn sync_file_returns_error_without_panicking_when_source_is_missing() {
    let error = sync_file(
        manager_with_templates(Vec::new()),
        "settings.txt".to_string(),
        None,
        None,
        None,
        None,
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("A source is required"));
}

#[test]
fn update_file_reports_missing_repo_config() {
    let destination = tempfile::tempdir().unwrap();

    let error = update_file(
        "settings.txt",
        "contents".to_string(),
        destination.path().to_str().unwrap(),
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("Could not open local repo config file"));
}

#[test]
fn update_file_reports_malformed_repo_config() {
    let destination = tempfile::tempdir().unwrap();
    fs::write(destination.path().join(".omni.yaml"), "dirs: [").unwrap();

    let error = update_file(
        "settings.txt",
        "contents".to_string(),
        destination.path().to_str().unwrap(),
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("Error parsing repo config YAML file"));
}

#[test]
fn update_file_creates_parent_directories_for_nested_targets() {
    let destination = tempfile::tempdir().unwrap();
    write_repo_config(destination.path(), &["repo-a", "repo-b"]);

    update_file(
        "nested/config/settings.toml",
        "contents".to_string(),
        destination.path().to_str().unwrap(),
    )
    .unwrap();

    assert_eq!(
        fs::read_to_string(
            destination
                .path()
                .join("repo-a/nested/config/settings.toml")
        )
        .unwrap(),
        "contents"
    );
    assert_eq!(
        fs::read_to_string(
            destination
                .path()
                .join("repo-b/nested/config/settings.toml")
        )
        .unwrap(),
        "contents"
    );
}

#[test]
fn update_file_attempts_all_repositories_and_aggregates_write_errors() {
    let destination = tempfile::tempdir().unwrap();
    write_repo_config_without_roots(destination.path(), &["good", "blocked"]);
    fs::create_dir(destination.path().join("good")).unwrap();
    fs::write(
        destination.path().join("blocked"),
        "a file, not a directory",
    )
    .unwrap();

    let error = update_file(
        "nested/settings.txt",
        "contents".to_string(),
        destination.path().to_str().unwrap(),
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("blocked"));
    assert_eq!(
        fs::read_to_string(destination.path().join("good/nested/settings.txt")).unwrap(),
        "contents"
    );
}

#[test]
fn update_file_reports_missing_repo_and_attempts_valid_repositories() {
    let destination = tempfile::tempdir().unwrap();
    write_repo_config_without_roots(destination.path(), &["valid", "missing"]);
    fs::create_dir(destination.path().join("valid")).unwrap();

    let error = update_file(
        "settings.txt",
        "contents".to_string(),
        destination.path().to_str().unwrap(),
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("missing"));
    assert_eq!(
        fs::read_to_string(destination.path().join("valid/settings.txt")).unwrap(),
        "contents"
    );
}

#[test]
fn update_file_rejects_traversal_in_repo_dir_and_attempts_valid_repositories() {
    let destination = tempfile::tempdir().unwrap();
    write_repo_config_without_roots(destination.path(), &["../outside", "valid"]);
    fs::create_dir(destination.path().join("valid")).unwrap();

    let error = update_file(
        "settings.txt",
        "contents".to_string(),
        destination.path().to_str().unwrap(),
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("repository directory"));
    assert_eq!(
        fs::read_to_string(destination.path().join("valid/settings.txt")).unwrap(),
        "contents"
    );
}

#[test]
fn update_file_rejects_traversal_in_target_filename() {
    let destination = tempfile::tempdir().unwrap();
    write_repo_config(destination.path(), &["repo"]);

    let error = update_file(
        "../escaped.txt",
        "contents".to_string(),
        destination.path().to_str().unwrap(),
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("target file"));
}

#[test]
fn update_file_rejects_absolute_target_filename() {
    let destination = tempfile::tempdir().unwrap();
    write_repo_config(destination.path(), &["repo"]);

    let error = update_file(
        "/tmp/escaped.txt",
        "contents".to_string(),
        destination.path().to_str().unwrap(),
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("target file"));
}

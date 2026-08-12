use super::*;
use crate::config::parser::{Config, Template, TemplateType};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tempfile::tempdir;

const TEST_HTTP_TIMEOUT: Duration = Duration::from_secs(2);

fn config_with_templates(templates: Vec<Template>) -> GlobalConfigManager {
    GlobalConfigManager::new(Config {
        repositories: vec![],
        templates,
    })
}

fn file_template(url: &str, dest: &str, tags: &[&str]) -> Template {
    Template {
        name: "template".to_owned(),
        id: "template-id".to_owned(),
        url: url.to_owned(),
        kind: TemplateType::File,
        dest: Some(dest.to_owned()),
        tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        included_files: None,
    }
}

fn spawn_http_server(response: Vec<u8>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + TEST_HTTP_TIMEOUT;
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        panic!("timed out waiting for HTTP client connection");
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("failed accepting HTTP client connection: {error}"),
            }
        };

        stream.set_read_timeout(Some(TEST_HTTP_TIMEOUT)).unwrap();
        stream.set_write_timeout(Some(TEST_HTTP_TIMEOUT)).unwrap();
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request);
        stream.write_all(&response).unwrap();
    });

    (format!("http://{address}/template.txt"), server)
}

fn http_response(status: u16, reason: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

#[test]
fn new_repo_creates_directory_and_deduplicates_default_tag() {
    let root = tempdir().unwrap();
    let observed_tags = Arc::new(Mutex::new(Vec::new()));
    let tags_for_copy = Arc::clone(&observed_tags);
    let initialized = Arc::new(Mutex::new(false));
    let initialized_for_init = Arc::clone(&initialized);

    new_repo_with(
        config_with_templates(vec![]),
        Some(vec!["default".to_owned(), "default".to_owned()]),
        Some(root.path().to_string_lossy().into_owned()),
        "repo".to_owned(),
        move |_cfg, tags, _dest| {
            tags_for_copy.lock().unwrap().extend(tags.iter().cloned());
            Ok(())
        },
        move |dest| {
            *initialized_for_init.lock().unwrap() = dest.is_dir();
            Ok(())
        },
    )
    .unwrap();

    assert!(root.path().join("repo").is_dir());
    let tags = observed_tags.lock().unwrap();
    assert_eq!(tags.iter().filter(|tag| *tag == "default").count(), 1);
    assert!(*initialized.lock().unwrap());
}

#[test]
fn new_repo_returns_error_when_destination_exists() {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join("repo")).unwrap();

    let result = new_repo_with(
        config_with_templates(vec![]),
        None,
        Some(root.path().to_string_lossy().into_owned()),
        "repo".to_owned(),
        |_cfg, _tags, _dest| Ok(()),
        |_dest| Ok(()),
    );

    assert!(result.is_err());
}

#[test]
fn new_repo_propagates_copy_error() {
    let root = tempdir().unwrap();

    let result = new_repo_with(
        config_with_templates(vec![]),
        None,
        Some(root.path().to_string_lossy().into_owned()),
        "repo".to_owned(),
        |_cfg, _tags, _dest| Err("copy failed".into()),
        |_dest| Ok(()),
    );

    let error = result.unwrap_err().to_string();
    assert!(error.contains("copy failed"));
}

#[test]
fn new_repo_propagates_init_error() {
    let root = tempdir().unwrap();

    let result = new_repo_with(
        config_with_templates(vec![]),
        None,
        Some(root.path().to_string_lossy().into_owned()),
        "repo".to_owned(),
        |_cfg, _tags, _dest| Ok(()),
        |_dest| Err("init failed".into()),
    );

    let error = result.unwrap_err().to_string();
    assert!(error.contains("init failed"));
}

#[test]
fn new_repo_rejects_absolute_and_parent_directory_names() {
    let root = tempdir().unwrap();
    let absolute_name = root.path().join("escape").to_string_lossy().into_owned();

    for name in [absolute_name, "../escape".to_owned()] {
        let result = new_repo_with(
            config_with_templates(vec![]),
            None,
            Some(root.path().to_string_lossy().into_owned()),
            name,
            |_cfg, _tags, _dest| Ok(()),
            |_dest| Ok(()),
        );

        let error = result.unwrap_err().to_string();
        assert!(error.contains("repository name"));
        assert!(!root.path().join("escape").exists());
    }
}

#[test]
fn copy_templates_creates_nested_template_destination() {
    let root = tempdir().unwrap();
    let cfg = config_with_templates(vec![file_template(
        "https://example.invalid/template.txt",
        "nested/deep",
        &["default"],
    )]);

    copy_templates_with(
        &cfg,
        &["default".to_owned()],
        root.path(),
        |_url| Ok("template body".to_owned()),
        |path, body| fs::write(path, body).map_err(|error| error.to_string()),
    )
    .unwrap();

    assert_eq!(
        fs::read_to_string(root.path().join("nested/deep/template.txt")).unwrap(),
        "template body"
    );
}

#[test]
fn copy_templates_rejects_absolute_and_parent_directory_destinations() {
    let root = tempdir().unwrap();
    let absolute_destination = root.path().join("escape").to_string_lossy().into_owned();

    for destination in [absolute_destination, "../escape".to_owned()] {
        let cfg = config_with_templates(vec![file_template(
            "https://example.invalid/template.txt",
            &destination,
            &["default"],
        )]);
        let fetch_calls = Arc::new(AtomicUsize::new(0));
        let fetch_calls_for_fetcher = Arc::clone(&fetch_calls);

        let result = copy_templates_with(
            &cfg,
            &["default".to_owned()],
            root.path(),
            move |_url| {
                fetch_calls_for_fetcher.fetch_add(1, Ordering::Relaxed);
                Ok("unexpected".to_owned())
            },
            |_path, _body| Ok(()),
        );

        let error = result.unwrap_err().to_string();
        assert!(error.contains("template destination"));
        assert_eq!(fetch_calls.load(Ordering::Relaxed), 0);
        assert!(!root.path().join("escape/template.txt").exists());
    }
}

#[test]
fn copy_templates_with_no_templates_does_not_call_fetcher() {
    let root = tempdir().unwrap();
    let cfg = config_with_templates(vec![]);
    let fetch_calls = Arc::new(AtomicUsize::new(0));
    let fetch_calls_for_fetcher = Arc::clone(&fetch_calls);

    copy_templates_with(
        &cfg,
        &["default".to_owned()],
        root.path(),
        move |_url| {
            fetch_calls_for_fetcher.fetch_add(1, Ordering::Relaxed);
            Err("fetch should not be called".to_owned())
        },
        |_path, _body| Err("write should not be called".to_owned()),
    )
    .unwrap();

    assert_eq!(fetch_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn copy_templates_returns_aggregate_fetch_errors() {
    let root = tempdir().unwrap();
    let cfg = config_with_templates(vec![
        file_template("https://example.invalid/one.txt", "", &["default"]),
        file_template("https://example.invalid/two.txt", "", &["default"]),
    ]);

    let result = copy_templates_with(
        &cfg,
        &["default".to_owned()],
        root.path(),
        |url| Err(format!("fetch failed for {url}")),
        |_path, _body| Ok(()),
    );

    let error = result.unwrap_err().to_string();
    assert!(error.contains("2 template(s) failed to copy"));
    assert!(error.contains("one.txt"));
    assert!(error.contains("two.txt"));
}

#[test]
fn copy_templates_returns_write_error() {
    let root = tempdir().unwrap();
    let cfg = config_with_templates(vec![file_template(
        "https://example.invalid/template.txt",
        "",
        &["default"],
    )]);

    let result = copy_templates_with(
        &cfg,
        &["default".to_owned()],
        root.path(),
        |_url| Ok("template body".to_owned()),
        |_path, _body| Err("disk full".to_owned()),
    );

    let error = result.unwrap_err().to_string();
    assert!(error.contains("failed writing"));
    assert!(error.contains("disk full"));
}

#[test]
fn copy_templates_returns_error_for_missing_filename() {
    let root = tempdir().unwrap();
    let cfg = config_with_templates(vec![file_template(
        "https://example.invalid/",
        "",
        &["default"],
    )]);
    let fetch_calls = Arc::new(AtomicUsize::new(0));
    let fetch_calls_for_fetcher = Arc::clone(&fetch_calls);

    let result = copy_templates_with(
        &cfg,
        &["default".to_owned()],
        root.path(),
        move |_url| {
            fetch_calls_for_fetcher.fetch_add(1, Ordering::Relaxed);
            Ok("unexpected".to_owned())
        },
        |_path, _body| Ok(()),
    );

    let error = result.unwrap_err().to_string();
    assert!(error.contains("does not contain a filename"));
    assert_eq!(fetch_calls.load(Ordering::Relaxed), 0);
}

#[test]
fn copy_templates_writes_http_200_response_to_file() {
    let root = tempdir().unwrap();
    let (url, server) = spawn_http_server(http_response(200, "OK", "template body"));
    let cfg = config_with_templates(vec![file_template(&url, "nested", &["default"])]);

    let result = copy_templates(&cfg, &["default".to_owned()], root.path());
    server.join().unwrap();
    result.unwrap();

    assert_eq!(
        fs::read_to_string(root.path().join("nested/template.txt")).unwrap(),
        "template body"
    );
}

#[test]
fn init_repo_returns_command_error() {
    let root = tempdir().unwrap();
    let result = init_repo_with(root.path(), |_dest| Err("git failed".to_owned()));

    let error = result.unwrap_err().to_string();
    assert!(error.contains("git failed"));
    assert!(error.contains("initialising repository"));
}

#[test]
fn fetch_template_rejects_error_status() {
    let (url, server) = spawn_http_server(http_response(500, "Internal Server Error", "nope"));
    let client = blocking::Client::builder()
        .no_proxy()
        .timeout(TEST_HTTP_TIMEOUT)
        .build()
        .unwrap();

    let result = fetch_template_with_client(&url, &client);
    server.join().unwrap();

    assert!(result.is_err());
    assert!(result.unwrap_err().contains("error status"));
}

use std::{
    fs,
    io::Error,
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use prettytable::{Table, row};
use rayon::iter::ParallelIterator;
use rayon::prelude::IntoParallelRefIterator;
use reqwest::blocking;

use crate::config::{
    manager::{GlobalConfigManager, RepoConfigManager},
    parser::{RepoConfig, Template, TemplateType},
};
use crate::util::utilities::join_relative;

const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

pub fn sync_file(
    cfg_mgr: GlobalConfigManager,
    file: String,
    url: Option<String>,
    template_id: Option<String>,
    destination: Option<String>,
    source_file: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = blocking::Client::builder().timeout(FETCH_TIMEOUT).build()?;
    sync_file_with_client(
        cfg_mgr,
        file,
        url,
        template_id,
        destination,
        source_file,
        &client,
    )
}

fn sync_file_with_client(
    cfg_mgr: GlobalConfigManager,
    file: String,
    url: Option<String>,
    template_id: Option<String>,
    destination: Option<String>,
    source_file: Option<String>,
    client: &blocking::Client,
) -> Result<(), Box<dyn std::error::Error>> {
    let dest = destination.unwrap_or_else(|| ".".to_string());
    info!("Destination is: {}", &dest);
    info!("File: {}", &file);

    let source = select_source(url, template_id, source_file).map_err(Error::other)?;
    let template_contents = match source {
        SourceSelector::Url(url) => {
            info!("Url to fetch template from is: {}", &url);
            fetch_template_with_client(client, &url)?
        }
        SourceSelector::TemplateId(template_id) => {
            let url = find_url_by_id(&cfg_mgr.config.templates, &template_id).ok_or_else(|| {
                Error::other(format!("Could not find template with id '{template_id}'."))
            })?;

            info!("Url to fetch template from is: {}", &url);
            fetch_template_with_client(client, &url)?
        }
        SourceSelector::LocalFile(source_file) => {
            let source_path = join_relative(Path::new(&dest), &source_file, "local source file")
                .map_err(|e| {
                    Error::other(format!(
                        "Could not read local source file '{}': {}",
                        source_file, e
                    ))
                })?;
            fs::read_to_string(&source_path).map_err(|e| {
                Error::other(format!(
                    "Could not read local source file '{}': {}",
                    source_path.display(),
                    e
                ))
            })?
        }
    };

    update_file(&file, template_contents, &dest)?;
    info!("Updated {} successfully across repositories.", &file);

    Ok(())
}

pub fn fetch_template(url: &str) -> Result<String, Box<dyn std::error::Error>> {
    let client = blocking::Client::builder().timeout(FETCH_TIMEOUT).build()?;
    fetch_template_with_client(&client, url)
}

fn fetch_template_with_client(
    client: &blocking::Client,
    url: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let response = client.get(url).send()?.error_for_status()?;
    Ok(response.text()?)
}

pub fn update_file(
    file_name: &str,
    contents: String,
    dest: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let config_file = Path::new(dest).join(".omni.yaml");
    info!("Using config file: {}", config_file.display());

    let repo_cfg_file = std::fs::File::open(&config_file).map_err(|e| {
        format!(
            "Could not open local repo config file: {}, {}",
            config_file.display(),
            e
        )
    })?;

    let config: RepoConfig = yaml_serde::from_reader(repo_cfg_file)
        .map_err(|e| format!("Error parsing repo config YAML file: {}", e))?;

    let rpc = RepoConfigManager::new(config);

    let dirs = rpc.get_dirs();

    let num_tasks = dirs.len();
    let completed_tasks = AtomicUsize::new(0);
    let progress_bar = ProgressBar::new(num_tasks as u64)
        .with_style(
          ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}")?
                .progress_chars("#>-"),
        );

    let result: Vec<Result<(), Error>> = dirs
        .par_iter()
        .map(|dir: &String| {
            let file_save: Result<(), Error> = match (|| {
                let repo_root = join_relative(Path::new(dest), dir, "repository directory")?;
                let metadata = fs::metadata(&repo_root).map_err(|e| {
                    Error::other(format!(
                        "Repository directory {} is not available: {}",
                        repo_root.display(),
                        e
                    ))
                })?;
                if !metadata.is_dir() {
                    return Err(Error::other(format!(
                        "Repository path {} is not a directory",
                        repo_root.display()
                    )));
                }

                let filename = join_relative(&repo_root, file_name, "target file")?;
                if let Some(parent) = filename.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&filename, &contents).map(|_| filename)
            })() {
                Ok(_) => {
                    info!("File {} has been written", dir);
                    Ok(())
                }
                Err(e) => {
                    info!("Failed saving file {} to disk. {}", dir, e);

                    Err(Error::other(format!(
                        "Failed to save file for repository {}: {}",
                        dir, e
                    )))
                }
            };

            // Each repository contributes exactly one completed task, regardless of outcome.
            progress_bar.inc(1);
            let completed = completed_tasks.fetch_add(1, Ordering::Relaxed);
            if completed + 1 == num_tasks {
                progress_bar.finish_with_message("All tasks completed.");
            }

            file_save
        })
        .collect();

    let mut table = Table::new();
    table.add_row(row!["No", "Fail", "Error Message"]);

    let mut error_count = 0;
    for res in &result {
        match res {
            Ok(_out) => {}
            Err(e) => {
                error_count += 1;
                table.add_row(row![error_count, "true", e.to_string()]);
            }
        }
    }

    if error_count > 0 {
        println!();
        println!();
        println!();
        table.printstd();

        let errors = result
            .iter()
            .filter_map(|res| res.as_ref().err().map(ToString::to_string))
            .collect::<Vec<_>>();
        return Err(Error::other(format!(
            "Failed to update {} of {} repositories: {}",
            error_count,
            num_tasks,
            errors.join("; ")
        ))
        .into());
    }

    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum SourceSelector {
    Url(String),
    TemplateId(String),
    LocalFile(String),
}

fn select_source(
    url: Option<String>,
    template_id: Option<String>,
    source_file: Option<String>,
) -> Result<SourceSelector, String> {
    match (url, template_id, source_file) {
        (Some(url), None, None) => Ok(SourceSelector::Url(url)),
        (None, Some(template_id), None) => Ok(SourceSelector::TemplateId(template_id)),
        (None, None, Some(source_file)) => Ok(SourceSelector::LocalFile(source_file)),
        (None, None, None) => Err(
            "A source is required; pass exactly one of --url, --template-file, or --source-file."
                .to_string(),
        ),
        _ => Err(
            "Conflicting template sources; pass only one of --url, --template-file, or --source-file."
                .to_string(),
        ),
    }
}

fn find_url_by_id(templates: &[Template], id: &str) -> Option<String> {
    templates
        .iter()
        .filter_map(|t| match &t.kind {
            TemplateType::File if t.id == id => Some(t.url.clone()),
            TemplateType::Dir => t.included_files.as_ref().and_then(|files| {
                files.iter().find_map(|file| {
                    if file.id == id {
                        Some(format!(
                            "{}/{}",
                            t.url.trim_end_matches('/'),
                            file.file_name.trim_start_matches('/')
                        ))
                    } else {
                        None
                    }
                })
            }),
            _ => None,
        })
        .next()
}

#[cfg(test)]
#[path = "synchronization_tests.rs"]
mod tests;

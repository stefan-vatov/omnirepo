use std::{
    fs::{self},
    io::Error,
    sync::atomic::{AtomicUsize, Ordering},
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

pub fn sync_file(
    cfg_mgr: GlobalConfigManager,
    file: String,
    url: Option<String>,
    template_id: Option<String>,
    destination: Option<String>,
    source_file: Option<String>,
) {
    let dest = destination.unwrap_or(".".into());
    info!("Destination is: {}", &dest);
    info!("File: {}", &file);

    let url = match (url, template_id) {
        (Some(url), _) => Some(url),
        (None, Some(template_id)) => find_url_by_id(&cfg_mgr.config.templates, &template_id),
        _ => None,
    };

    if url.is_none() && source_file.is_none() {
        panic!("Could not find url or source file for template.")
    }

    let template_contents = match (url, source_file) {
        (Some(_), Some(_)) => {
            panic!("Pass only one template source flag");
        }
        (Some(url), _) => {
            let valid_url = url;

            info!("Url to fetch template from is: {}", &valid_url);

            fetch_template(&valid_url)
        }
        (None, Some(source_file)) => {
            match fs::read_to_string(format!("{}/{}", &dest, &source_file)) {
                Ok(contents) => Some(contents),
                Err(e) => {
                    info!("{}", e);

                    None
                }
            }
        }
        _ => None,
    };

    if template_contents.is_none() {
        panic!("Could not fetch contents for template");
    }

    match update_file(&file, template_contents.unwrap(), &dest) {
        Ok(_) => info!("Updated {} sucessfully across repositories.", &file),
        Err(e) => info!("Error updating file: {:?}", &e),
    };
}

pub fn fetch_template(url: &str) -> Option<String> {
    match blocking::get(url) {
        Ok(res) => match res.text() {
            Ok(text) => Some(text),
            Err(_) => {
                info!("Failed extracting contents from response of {}", url);

                None
            }
        },
        Err(e) => {
            info!("Failed fetching {}. {}", url, e);

            None
        }
    }
}

pub fn update_file(
    file_name: &str,
    contents: String,
    dest: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let config_file = format!("{}/.omni.yaml", &dest);
    info!("Using config file: {}", &config_file);

    let repo_cfg_file = std::fs::File::open(&config_file).map_err(|e| {
        format!(
            "Could not open local repo config file: {}, {}",
            &config_file, e
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
            let filename = format!("{}/{}/{}", dest, &dir, &file_name);

            let file_save: Result<(), Error> = match fs::write(&filename, &contents) {
                Ok(_) => {
                    info!("File {} has been written", &filename);
                    // Update progress bar
                    progress_bar.inc(1);
                    let completed = completed_tasks.fetch_add(1, Ordering::Relaxed);
                    if completed + 1 == num_tasks {
                        progress_bar.finish_with_message("All tasks completed.");
                    }

                    Ok(())
                }
                Err(e) => {
                    info!("Failed saving file {} to disk. {}", &filename, e);

                    Err(Error::other(format!(
                        "Failed to save file {}: {}",
                        &filename, &e
                    )))
                }
            };

            // Update progress bar
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
    }

    Ok(())
}

fn find_url_by_id(templates: &[Template], id: &str) -> Option<String> {
    templates
        .iter()
        .filter_map(|t| match t.kind {
            TemplateType::File if t.id == id => Some(t.url.clone()),
            TemplateType::Dir => t.included_files.as_ref().and_then(|files| {
                files.iter().find_map(|file| {
                    if file.id == id {
                        Some(format!("{}/{}", t.url, file.file_name))
                    } else {
                        None
                    }
                })
            }),
            _ => None,
        })
        .next()
}

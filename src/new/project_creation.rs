use duct::cmd;
use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use rayon::prelude::*;
use reqwest::blocking;
use std::{
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use crate::{
    config::manager::GlobalConfigManager,
    util::utilities::{
        dedupe_vec_string, filename_from_url, join_relative, template_and_dest_from_tags,
    },
};

const TEMPLATE_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub fn new_repo(
    cfg_mgr: GlobalConfigManager,
    tags: Option<Vec<String>>,
    destination: Option<String>,
    name: String,
) -> Result<(), Box<dyn Error>> {
    new_repo_with(cfg_mgr, tags, destination, name, copy_templates, init_repo)
}

fn new_repo_with<C, I>(
    cfg_mgr: GlobalConfigManager,
    tags: Option<Vec<String>>,
    destination: Option<String>,
    name: String,
    copy: C,
    init: I,
) -> Result<(), Box<dyn Error>>
where
    C: Fn(&GlobalConfigManager, &[String], &Path) -> Result<(), Box<dyn Error>>,
    I: Fn(&Path) -> Result<(), Box<dyn Error>>,
{
    let dest = destination.unwrap_or_else(|| ".".to_owned());
    let mut valid_tags = tags.unwrap_or_default();

    info!("Creating new repo: {:?}", &name);
    info!("Creating to: {:?}", &dest);

    let dir_to_create = join_relative(Path::new(&dest), &name, "repository name")?;

    info!("Creating directory: {:?}", &dir_to_create);
    fs::create_dir(&dir_to_create)?;

    valid_tags.push("default".to_owned());
    let unique_tags = dedupe_vec_string(valid_tags);

    copy(&cfg_mgr, &unique_tags, &dir_to_create)?;
    info!("Templates copied to {:?}", &dir_to_create);

    init(&dir_to_create)?;
    info!("Git repo initialized in {:?}", &dir_to_create);

    Ok(())
}

pub fn copy_templates(
    cfg_mgr: &GlobalConfigManager,
    tags: &[String],
    dest: &Path,
) -> Result<(), Box<dyn Error>> {
    copy_templates_with(cfg_mgr, tags, dest, fetch_template, write_template)
}

fn copy_templates_with<F, W>(
    cfg_mgr: &GlobalConfigManager,
    tags: &[String],
    dest: &Path,
    fetch: F,
    write: W,
) -> Result<(), Box<dyn Error>>
where
    F: Fn(&str) -> Result<String, String> + Sync,
    W: Fn(&Path, &str) -> Result<(), String> + Sync,
{
    let template_pairs = template_and_dest_from_tags(tags, cfg_mgr);
    let num_tasks = template_pairs.len();
    let completed_tasks = AtomicUsize::new(0);
    let progress_style = ProgressStyle::default_bar()
        .template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}",
        )?
        .progress_chars("#>-");
    let progress_bar = ProgressBar::new(num_tasks as u64).with_style(progress_style);

    let results: Vec<Result<(), String>> = template_pairs
        .par_iter()
        .map(|pair| {
            let result = copy_one_template(pair, dest, &fetch, &write);

            // Every item, including failed items, advances the progress bar.
            progress_bar.inc(1);
            let completed = completed_tasks.fetch_add(1, Ordering::Relaxed) + 1;
            if completed == num_tasks {
                progress_bar.finish_with_message("All tasks completed.");
            }

            result.map_err(|error| format!("{}: {}", pair.0, error))
        })
        .collect();

    let failures: Vec<String> = results.into_iter().filter_map(Result::err).collect();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(Box::new(TemplateCopyError { failures }))
    }
}

fn copy_one_template<F, W>(
    pair: &(String, String),
    dest: &Path,
    fetch: &F,
    write: &W,
) -> Result<(), String>
where
    F: Fn(&str) -> Result<String, String> + Sync,
    W: Fn(&Path, &str) -> Result<(), String> + Sync,
{
    let filename = filename_from_url(&pair.0);
    if filename.is_empty() {
        return Err("URL does not contain a filename".to_owned());
    }

    let file_dir = join_relative(dest, &pair.1, "template destination")
        .map_err(|error| format!("invalid template destination: {error}"))?;
    let body = fetch(&pair.0).map_err(|error| format!("failed fetching template: {error}"))?;
    fs::create_dir_all(&file_dir)
        .map_err(|error| format!("failed creating directory {}: {error}", file_dir.display()))?;

    let filename = file_dir.join(filename);
    write(&filename, &body)
        .map_err(|error| format!("failed writing {}: {error}", filename.display()))?;

    info!("File {} has been written", filename.display());
    Ok(())
}

fn fetch_template(url: &str) -> Result<String, String> {
    let client = blocking::Client::builder()
        .timeout(TEMPLATE_REQUEST_TIMEOUT)
        .build()
        .map_err(|error| format!("failed building HTTP client: {error}"))?;

    fetch_template_with_client(url, &client)
}

fn fetch_template_with_client(url: &str, client: &blocking::Client) -> Result<String, String> {
    let response = client
        .get(url)
        .send()
        .map_err(|error| format!("request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("request returned an error status: {error}"))?;

    response
        .text()
        .map_err(|error| format!("failed extracting response contents: {error}"))
}

fn write_template(path: &Path, body: &str) -> Result<(), String> {
    fs::write(path, body).map_err(|error| error.to_string())
}

pub fn init_repo(dest: &Path) -> Result<(), Box<dyn Error>> {
    init_repo_with(dest, |path| {
        cmd!("git", "init")
            .dir(path)
            .stdout_null()
            .stderr_null()
            .run()
            .map(|_| ())
            .map_err(|error| error.to_string())
    })?;

    println!();
    println!();
    println!("Repository created at {:?}", dest);
    Ok(())
}

fn init_repo_with<F>(dest: &Path, command: F) -> Result<(), Box<dyn Error>>
where
    F: Fn(&Path) -> Result<(), String>,
{
    command(dest).map_err(|message| {
        Box::new(RepositoryInitError {
            destination: dest.to_path_buf(),
            message,
        }) as Box<dyn Error>
    })
}

#[derive(Debug)]
struct TemplateCopyError {
    failures: Vec<String>,
}

impl fmt::Display for TemplateCopyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} template(s) failed to copy: {}",
            self.failures.len(),
            self.failures.join("; ")
        )
    }
}

impl Error for TemplateCopyError {}

#[derive(Debug)]
struct RepositoryInitError {
    destination: PathBuf,
    message: String,
}

impl fmt::Display for RepositoryInitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Error initialising repository at {:?}: {}",
            self.destination, self.message
        )
    }
}

impl Error for RepositoryInitError {}

#[cfg(test)]
#[path = "project_creation_tests.rs"]
mod tests;

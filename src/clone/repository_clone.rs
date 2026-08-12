use std::{collections::HashSet, fs, io::Error, path::PathBuf, process::Output};

use duct::cmd;
use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use prettytable::{Table, row};
use rayon::prelude::*;

use crate::{
    config::{manager::GlobalConfigManager, parser::RepoConfig},
    util::utilities::join_relative,
};

type CloneTarget = (String, String);

/// Return each requested repository and its configured destination once.
///
/// The order is intentionally driven by the order of `tags`, then by the
/// order of repositories in the global config. A URL and destination are kept
/// together while deduplicating so that repositories with the same URL but
/// different destinations remain distinct.
fn clone_targets(tags: &[String], cfg_mgr: &GlobalConfigManager) -> Vec<CloneTarget> {
    let mut seen = HashSet::new();
    let mut targets = Vec::new();

    for tag in tags {
        for repository in &cfg_mgr.config.repositories {
            if repository
                .tags
                .iter()
                .any(|repository_tag| repository_tag == tag)
            {
                let target = (repository.url.clone(), repository.dest.clone());
                if seen.insert(target.clone()) {
                    targets.push(target);
                }
            }
        }
    }

    targets
}

pub fn clone_repo(
    cfg_mgr: GlobalConfigManager,
    tags: &[String],
    destination: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let targets = clone_targets(tags, &cfg_mgr);
    let dest = PathBuf::from(destination.unwrap_or_else(|| ".".to_owned()));

    info!("Cloning repos: {:?}", &targets);
    info!("Cloning to: {:?}", &dest);

    let resolved_targets: Vec<(String, PathBuf)> = targets
        .iter()
        .map(|(repo, repo_dest)| {
            join_relative(&dest, repo_dest, "Repository destination")
                .map(|resolved_dest| (repo.clone(), resolved_dest))
        })
        .collect::<std::io::Result<_>>()?;

    let mut seen_destinations = HashSet::with_capacity(resolved_targets.len());
    for (_, repo_dest) in &resolved_targets {
        if !seen_destinations.insert(repo_dest) {
            return Err(Box::new(Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "Repository destination is configured more than once: {}",
                    repo_dest.display()
                ),
            )));
        }
    }

    let num_tasks = targets.len();
    let progress_bar = ProgressBar::new(num_tasks as u64).with_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}",
            )?
            .progress_chars("#>-"),
    );

    let clone_results: Vec<Result<Output, String>> = resolved_targets
        .par_iter()
        .map(|(repo, repo_dest)| {
            let output = cmd!("git", "clone", repo, repo_dest.as_path())
                .stdout_null()
                .stderr_null()
                .run();

            progress_bar.inc(1);

            output.map_err(|error| {
                format!(
                    "Error cloning repo {} to {}: {}",
                    repo,
                    repo_dest.display(),
                    error
                )
            })
        })
        .collect();

    progress_bar.finish_with_message("All tasks completed.");

    let errors: Vec<String> = clone_results.into_iter().filter_map(Result::err).collect();

    if !errors.is_empty() {
        let mut table = Table::new();
        table.add_row(row!["No", "Fail", "Error Message"]);
        for (number, error) in errors.iter().enumerate() {
            table.add_row(row![number + 1, "true", error]);
        }

        println!();
        table.printstd();

        return Err(Box::new(Error::other(format!(
            "{} repository clone(s) failed:\n{}",
            errors.len(),
            errors.join("\n")
        ))));
    }

    let dirs = targets
        .into_iter()
        .map(|(_, repo_dest)| repo_dest)
        .collect();
    let rpc = yaml_serde::to_string(&RepoConfig::new(dirs))?;
    fs::write(dest.join(".omni.yaml"), rpc)?;

    Ok(())
}

#[cfg(test)]
#[path = "repository_clone_tests.rs"]
mod tests;

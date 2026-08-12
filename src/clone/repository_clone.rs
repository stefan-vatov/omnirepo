use std::{
    fs,
    io::Error,
    process::Output,
    sync::atomic::{AtomicUsize, Ordering},
};

use duct::cmd;
use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use prettytable::{Table, row};
use rayon::prelude::*;

use crate::{
    config::{manager::GlobalConfigManager, parser::RepoConfig},
    util::utilities::{get_dest_from_tags, get_repos_from_tags},
};

pub fn clone_repo(
    cfg_mgr: GlobalConfigManager,
    tags: &[String],
    destination: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let repos = get_repos_from_tags(tags, &cfg_mgr);
    let dests = get_dest_from_tags(tags, &cfg_mgr);

    info!("Cloning repos: {:?}", &repos);
    info!("Cloning to: {:?}", &dests);

    let dest = destination.unwrap_or(".".into());
    info!("Cloning to: {:?}", &dest);

    let num_tasks = repos.len();
    let completed_tasks = AtomicUsize::new(0);
    let progress_bar = ProgressBar::new(num_tasks as u64)
        .with_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}")?
                .progress_chars("#>-"),
        );

    let clone_result: Vec<Result<Output, Error>> = repos
        .par_iter()
        .map(|repo: &String| {
            let output = cmd!("git", "clone", repo)
                .dir(&dest)
                .stdout_null()
                .stderr_null()
                .run();

            // Update progress bar
            progress_bar.inc(1);
            let completed = completed_tasks.fetch_add(1, Ordering::Relaxed);
            if completed + 1 == num_tasks {
                progress_bar.finish_with_message("All tasks completed.");
            }

            output.map_err(|e| Error::other(format!("Error cloning repo: {}, {}", repo.clone(), e)))
        })
        .collect();

    let mut table = Table::new();
    table.add_row(row!["No", "Fail", "Error Message"]);

    let mut error_count = 0;
    for res in &clone_result {
        match res {
            Ok(_out) => (),
            Err(e) => {
                error_count += 1;
                table.add_row(row![error_count, "true", e.to_string()]);
            }
        }
    }

    let rpc = yaml_serde::to_string(&RepoConfig::new(dests))
        .expect("Failed to serialise local multirepo config.");
    fs::write(format!("{}/.omni.yaml", &dest), rpc)?;

    if error_count > 0 {
        println!();
        println!();
        println!();
        table.printstd();
    }

    Ok(())
}

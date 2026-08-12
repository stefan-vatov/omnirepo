use crate::{
    config::{manager::RepoConfigManager, parser::RepoConfig},
    util::utilities::join_relative,
};
use duct::cmd;
use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use rayon::prelude::*;
use std::{io::Error, path::Path, process::Output};

use prettytable::{Table, row};

pub fn run_command(
    command_string: String,
    destination: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let dest = destination.unwrap_or_else(|| ".".to_owned());
    let destination_path = Path::new(&dest);
    info!("Running command: {}", &command_string);

    let config_file = destination_path.join(".omni.yaml");
    info!("Using config file: {}", config_file.display());

    let file = std::fs::File::open(&config_file).map_err(|e| {
        format!(
            "Could not open local repo config file: {}, {}",
            config_file.display(),
            e
        )
    })?;

    let config: RepoConfig = yaml_serde::from_reader(file).map_err(|e| {
        format!(
            "Error parsing repo config YAML file {}: {}",
            config_file.display(),
            e
        )
    })?;

    let rpc = RepoConfigManager::new(config);

    let dirs = rpc.get_dirs();

    let num_tasks = dirs.len();
    let progress_bar = ProgressBar::new(num_tasks as u64)
        .with_style(
          ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}")?
                .progress_chars("#>-"),
        );

    let result: Vec<Result<Output, Error>> = dirs
        .par_iter()
        .map(|dir: &String| {
            let result = match join_relative(destination_path, dir, "Repository directory") {
                Ok(cmd_dir) => cmd("sh", &["-c", &command_string])
                    .dir(&cmd_dir)
                    .stdout_capture()
                    .stderr_capture()
                    .run()
                    .map_err(|e| {
                        Error::other(format!(
                            "Error running command in {}: {}",
                            cmd_dir.display(),
                            e
                        ))
                    }),
                Err(e) => Err(e),
            };

            // Each repository gets exactly one progress update, whether its
            // path is invalid, its command succeeds, fails, or cannot be
            // launched.
            progress_bar.inc(1);

            result
        })
        .collect();

    progress_bar.finish_with_message("All tasks completed.");

    let mut table = Table::new();
    table.add_row(row!["No", "Fail", "Error Message"]);

    let mut error_count = 0;
    let mut errors = Vec::new();
    for res in &result {
        match res {
            Ok(out) => {
                info!("{:?}", out.stdout);
                info!("{:?}", out.stderr);
            }
            Err(e) => {
                error_count += 1;
                let message = e.to_string();
                table.add_row(row![error_count, "true", message.clone()]);
                errors.push(message);
                info!("{:?}", e);
            }
        }
    }

    if error_count > 0 {
        println!();
        println!();
        println!();
        table.printstd();
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(Error::other(format!(
            "{} repository command(s) failed:\n{}",
            errors.len(),
            errors.join("\n")
        ))
        .into())
    }
}

#[cfg(test)]
#[path = "runners_tests.rs"]
mod tests;

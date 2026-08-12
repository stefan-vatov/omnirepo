use crate::config::{manager::RepoConfigManager, parser::RepoConfig};
use duct::cmd;
use indicatif::{ProgressBar, ProgressStyle};
use log::info;
use rayon::prelude::*;
use std::{
    io::Error,
    process::Output,
    sync::atomic::{AtomicUsize, Ordering},
};

use prettytable::{Table, row};

pub fn run_command(
    command_string: String,
    destination: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let dest = destination.unwrap_or(".".into());
    info!("Running command: {}", &command_string);

    let config_file = format!("{}/.omni.yaml", &dest);
    info!("Using config file: {}", &config_file);

    let file = std::fs::File::open(&config_file).map_err(|e| {
        format!(
            "Could not open local repo config file: {}, {}",
            &config_file, e
        )
    })?;

    let config: RepoConfig = yaml_serde::from_reader(file)
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

    let result: Vec<Result<Output, Error>> = dirs
        .par_iter()
        .map(|dir: &String| {
            let cmd_dir = format!("{}/{}", &dest, &dir);
            let output = cmd("sh", &["-c", &command_string])
                .dir(&cmd_dir)
                .stdout_capture()
                .stderr_capture()
                .run();

            // Update progress bar
            progress_bar.inc(1);
            let completed = completed_tasks.fetch_add(1, Ordering::Relaxed);
            if completed + 1 == num_tasks {
                progress_bar.finish_with_message("All tasks completed.");
            }

            output.map_err(|e| Error::other(format!("Error running command: {}, {}", &cmd_dir, e)))
        })
        .collect();

    let mut table = Table::new();
    table.add_row(row!["No", "Fail", "Error Message"]);

    let mut error_count = 0;
    for res in &result {
        match res {
            Ok(out) => {
                info!("{:?}", out.stdout);
                info!("{:?}", out.stderr);
            }
            Err(e) => {
                error_count += 1;
                table.add_row(row![error_count, "true", e]);
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

    Ok(())
}

use std::{
    collections::{HashMap, HashSet},
    error::Error,
    path::Path,
};

use crate::config::{manager::GlobalConfigManager, parser::Config};

pub fn get_repos_from_tags(tags: &[String], cfg_mgr: &GlobalConfigManager) -> Vec<String> {
    dedupe_vec_string(
        tags.iter()
            .flat_map(|tag| cfg_mgr.get_url_by_tag(tag))
            .collect(),
    )
}

pub fn get_dest_from_tags(tags: &[String], cfg_mgr: &GlobalConfigManager) -> Vec<String> {
    dedupe_vec_string(
        tags.iter()
            .flat_map(|tag| cfg_mgr.get_dest_by_tag(tag))
            .collect(),
    )
}

pub fn template_and_dest_from_tags(
    tags: &[String],
    cfg_mgr: &GlobalConfigManager,
) -> Vec<(String, String)> {
    dedupe_vec_tuple(
        tags.iter()
            .flat_map(|tag| cfg_mgr.template_and_dest(tag))
            .collect(),
    )
}

pub fn dedupe_vec_string(combined: Vec<String>) -> Vec<String> {
    let unique: HashSet<String> = combined.into_iter().collect();

    unique.into_iter().collect()
}

fn dedupe_vec_tuple(combined: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut unique_map = HashMap::new();

    for (key, value) in combined {
        unique_map.entry(key).or_insert(value);
    }

    unique_map.into_iter().collect()
}

pub fn load_config(config_location: &Path) -> Result<GlobalConfigManager, Box<dyn Error>> {
    let config = load_config_from_file(config_location)?;

    Ok(GlobalConfigManager::new(config))
}

pub fn load_config_default() -> Result<GlobalConfigManager, Box<dyn Error>> {
    let config_file = dirs::home_dir()
        .expect("Could not find home directory")
        .join(".omnirepo.yaml");

    let config_dir = dirs::home_dir()
        .expect("Could not find home directory")
        .join(".omnirepo/.omnirepo.yaml");

    if Path::new(&config_file).exists() {
        load_config(&config_file)
    } else if Path::new(&config_dir).exists() {
        load_config(&config_dir)
    } else {
        Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Default config file not found.",
        )))
    }
}

fn load_config_from_file(config_location: &Path) -> Result<Config, Box<dyn Error>> {
    let file = std::fs::File::open(config_location)
        .map_err(|e| format!("Could not open config file: {:?} {}", config_location, e))?;
    let config =
        yaml_serde::from_reader(file).map_err(|e| format!("Error parsing YAML file: {}", e))?;

    Ok(config)
}

pub fn filename_from_url(url: &str) -> &str {
    url.split('/').next_back().unwrap_or("")
}

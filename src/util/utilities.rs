use std::{
    collections::HashSet,
    error::Error,
    path::{Component, Path, PathBuf},
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
    let mut seen = HashSet::with_capacity(combined.len());
    let mut unique = Vec::with_capacity(combined.len());

    for value in combined {
        if seen.insert(value.clone()) {
            unique.push(value);
        }
    }

    unique
}

fn dedupe_vec_tuple(combined: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut seen = HashSet::with_capacity(combined.len());
    let mut unique = Vec::with_capacity(combined.len());

    for pair in combined {
        if seen.insert(pair.clone()) {
            unique.push(pair);
        }
    }

    unique
}

pub fn load_config(config_location: &Path) -> Result<GlobalConfigManager, Box<dyn Error>> {
    let config = load_config_from_file(&resolve_config_path(config_location))?;

    Ok(GlobalConfigManager::new(config))
}

pub fn load_config_default() -> Result<GlobalConfigManager, Box<dyn Error>> {
    let home_dir = dirs::home_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Could not find home directory",
        )
    })?;

    load_config_default_from_home(&home_dir)
}

pub fn default_config_paths(home_dir: &Path) -> [PathBuf; 2] {
    [
        home_dir.join(".omnirepo.yaml"),
        home_dir.join(".omnirepo/.omnirepo.yaml"),
    ]
}

fn load_config_default_from_home(home_dir: &Path) -> Result<GlobalConfigManager, Box<dyn Error>> {
    let [config_file, config_dir] = default_config_paths(home_dir);

    if config_file.is_file() {
        load_config(&config_file)
    } else if config_dir.is_file() {
        load_config(&config_dir)
    } else {
        Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Default config file not found.",
        )))
    }
}

fn resolve_config_path(config_location: &Path) -> PathBuf {
    if config_location.is_dir() {
        config_location.join(".omnirepo.yaml")
    } else {
        config_location.to_path_buf()
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

pub(crate) fn join_relative(
    base: &Path,
    relative: &str,
    description: &str,
) -> std::io::Result<PathBuf> {
    let path = Path::new(relative);

    if let Some(component) = path.components().find(|component| {
        matches!(
            component,
            Component::RootDir | Component::Prefix(_) | Component::ParentDir
        )
    }) {
        let reason = match component {
            Component::RootDir => "an absolute path",
            Component::Prefix(_) => "a path prefix",
            Component::ParentDir => "parent-directory traversal",
            Component::CurDir | Component::Normal(_) => unreachable!(),
        };

        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{description} must be a relative path without {reason}: {relative:?}"),
        ));
    }

    Ok(base.join(path))
}

#[cfg(test)]
#[path = "utilities_tests.rs"]
mod tests;

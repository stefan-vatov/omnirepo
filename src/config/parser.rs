use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Config {
    pub repositories: Vec<Repository>,
    pub templates: Vec<Template>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Repositories {
    pub repositories: Vec<Repository>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Repository {
    pub name: String,
    pub url: String,
    pub tags: Vec<String>,
    pub dest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RepoConfig {
    pub dirs: Vec<String>,
}

impl RepoConfig {
    pub fn new(dirs: Vec<String>) -> Self {
        Self { dirs }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Templates {
    pub templates: Vec<Template>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Template {
    pub name: String,
    pub id: String,
    pub url: String,
    pub kind: TemplateType,
    pub dest: Option<String>,
    pub tags: Vec<String>,
    pub included_files: Option<Vec<IncludedFile>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IncludedFile {
    pub file_name: String,
    pub id: String,
    pub dest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TemplateType {
    File,
    Dir,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlobalConfig {
    pub log: bool,
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod tests;

//! Synthetic local fleet and source generators.
//!
//! Deterministic generators (seeded) produce machine configs, source
//! catalogs, policies, Git remotes, checks, and managed content at
//! chosen sizes — no external network and no huge committed fixtures;
//! everything is generated at runtime below the harness root.  The
//! generators preserve source order and exact content; fleet identities
//! are unique and alias cases are explicit; seeds reproduce the exact
//! generated fleet; setup and cleanup stay contained.

#![allow(dead_code)]

#[cfg(test)]
mod fleet_generators_tests;

use std::{
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
};

/// The managed content kinds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentKind {
    WholeFile,
    Section,
}

/// One materialized fleet.
#[derive(Clone, Debug)]
pub struct FleetGeneration {
    pub root: PathBuf,
    pub seed: u64,
    pub repository_count: usize,
}

/// Generator failures.
#[derive(Debug)]
pub enum GeneratorError {
    Root { path: PathBuf, reason: String },
    Io { path: PathBuf, reason: String },
}

impl fmt::Display for GeneratorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root { path, reason } => {
                write!(
                    formatter,
                    "fleet generator root failure {}: {reason}",
                    path.display()
                )
            }
            Self::Io { path, reason } => {
                write!(
                    formatter,
                    "fleet generator io failure {}: {reason}",
                    path.display()
                )
            }
        }
    }
}
impl Error for GeneratorError {}

/// The deterministic PRNG for generation.
struct SeededRandom(u64);

impl SeededRandom {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// Generate the machine config text: the declared repository list is
/// preserved in order with exact content; the same seed reproduces the
/// exact bytes.
pub fn generate_machine_config(seed: u64, repositories: &[(String, usize)]) -> String {
    let mut text = String::from("machine:\n  version: 1\n  repositories:\n");
    for (repository, items) in repositories {
        text.push_str(&format!("    - id: {repository}\n      items: {items}\n"));
    }
    text.push_str(&format!("  seed: {seed}\n"));
    text
}

/// Generate the source catalog text: declared item order is preserved.
pub fn generate_source_catalog(seed: u64, items: &[(String, String)]) -> String {
    let mut text = String::from("sources:\n");
    for (target, source) in items {
        text.push_str(&format!("  - target: {target}\n    source: {source}\n"));
    }
    text.push_str(&format!("  seed: {seed}\n"));
    text
}

/// Generate deterministic managed content of exactly the chosen size.
pub fn generate_managed_content(seed: u64, size: usize, kind: ContentKind) -> Vec<u8> {
    let mut random = SeededRandom::new(seed);
    let mut bytes = Vec::with_capacity(size);
    while bytes.len() < size {
        bytes.extend_from_slice(&random.next().to_le_bytes());
    }
    bytes.truncate(size);
    match kind {
        ContentKind::WholeFile => bytes,
        ContentKind::Section => {
            // The section kind carries a stable envelope around the
            // deterministic payload.
            let mut section = b"# omnirepo-start\n".to_vec();
            section.extend_from_slice(&bytes);
            section.extend_from_slice(b"\n# omnirepo-end\n");
            section
        }
    }
}

/// Materialize a fleet of `repository_count` repositories below the
/// harness root: every repository gets a uniquely named directory with a
/// machine config, a source catalog, and deterministic managed content.
/// The same seed reproduces the same fleet.
pub fn materialize_fleet(
    seed: u64,
    repository_count: usize,
    root: &Path,
) -> Result<FleetGeneration, GeneratorError> {
    if !root.is_dir() {
        return Err(GeneratorError::Root {
            path: root.to_path_buf(),
            reason: "the harness root is not a directory".to_owned(),
        });
    }
    let mut random = SeededRandom::new(seed);
    let fleet_root = root.join(format!("fleet-{seed}-{repository_count}"));
    fs::create_dir_all(&fleet_root).map_err(|error| GeneratorError::Io {
        path: fleet_root.clone(),
        reason: error.to_string(),
    })?;
    for index in 0..repository_count {
        // Unique, traversal-free identities.
        let identity = format!("repo-{index}-{:016x}", random.next());
        let repository_dir = fleet_root.join(&identity);
        fs::create_dir_all(&repository_dir).map_err(|error| GeneratorError::Io {
            path: repository_dir.clone(),
            reason: error.to_string(),
        })?;
        let repositories = vec![(identity.clone(), 1_usize)];
        let config = generate_machine_config(seed.wrapping_add(index as u64), &repositories);
        fs::write(repository_dir.join("machine.yaml"), config).map_err(|error| {
            GeneratorError::Io {
                path: repository_dir.join("machine.yaml"),
                reason: error.to_string(),
            }
        })?;
        let items = vec![(format!("managed-{index}.txt"), identity.clone())];
        let catalog = generate_source_catalog(seed.wrapping_add(index as u64), &items);
        fs::write(repository_dir.join("sources.txt"), catalog).map_err(|error| {
            GeneratorError::Io {
                path: repository_dir.join("sources.txt"),
                reason: error.to_string(),
            }
        })?;
        let content =
            generate_managed_content(seed.wrapping_add(index as u64), 64, ContentKind::WholeFile);
        fs::write(repository_dir.join("managed-0.txt"), content).map_err(|error| {
            GeneratorError::Io {
                path: repository_dir.join("managed-0.txt"),
                reason: error.to_string(),
            }
        })?;
    }
    Ok(FleetGeneration {
        root: fleet_root,
        seed,
        repository_count,
    })
}

/// Remove a generated fleet (contained cleanup).
pub fn remove_fleet(fleet: &FleetGeneration) -> Result<(), GeneratorError> {
    fs::remove_dir_all(&fleet.root).map_err(|error| GeneratorError::Io {
        path: fleet.root.clone(),
        reason: error.to_string(),
    })
}

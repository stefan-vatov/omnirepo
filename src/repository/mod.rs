#![allow(dead_code)]

mod capture;
mod git_index;
mod manifest;
mod operation_commit;
mod operation_tree;
mod policy;
mod policy_loader;
mod revalidate;
mod state;

pub(crate) use state::RepositoryId;

#[cfg(test)]
mod capture_tests;

#[cfg(test)]
mod git_index_tests;

#[cfg(test)]
mod operation_commit_tests;

#[cfg(test)]
mod operation_tree_tests;

#[cfg(test)]
mod manifest_tests;

#[cfg(test)]
mod policy_loader_tests;

#[cfg(test)]
mod policy_tests;

#[cfg(test)]
mod state_tests;

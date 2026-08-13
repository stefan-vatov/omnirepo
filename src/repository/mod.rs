#![allow(dead_code)]

mod capture;
mod git_index;
mod manifest;
mod policy;
mod policy_loader;
mod revalidate;
mod state;

#[cfg(test)]
mod capture_tests;

#[cfg(test)]
mod git_index_tests;

#[cfg(test)]
mod manifest_tests;

#[cfg(test)]
mod policy_loader_tests;

#[cfg(test)]
mod policy_tests;

#[cfg(test)]
mod state_tests;

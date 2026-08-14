#![allow(dead_code)]

mod compare;
mod delimiters;
mod partial_scan;
mod representation;
mod section_append;
mod section_builder;
mod transaction;
mod whole_file;

pub(crate) use transaction::{TempCandidate, TransactionPlan};

#[cfg(test)]
pub(crate) use whole_file::{WholeFileOutcome, classify_whole_file};

#[cfg(test)]
pub(crate) use transaction::ParentDirectories;

#[cfg(test)]
mod compare_tests;

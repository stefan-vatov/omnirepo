#![allow(dead_code)]

mod compare;
mod delimiters;
mod partial_scan;
mod representation;
mod section_apply;
#[cfg(test)]
mod section_fixture_tests;
mod transaction;
mod whole_file;

pub(crate) use compare::CompareOutcome;
pub(crate) use delimiters::{DelimiterError, DelimiterSyntax, lookup_by_extension};
pub(crate) use partial_scan::{ScanOutcome, scan_sections, split_inclusive_lines};
pub(crate) use representation::{Representation, check_exact_representation};
pub(crate) use section_apply::{SectionWrite, apply_sections};
pub(crate) use transaction::{TempCandidate, TransactionPlan};

#[cfg(test)]
pub(crate) use representation::destination_equals_source;

#[cfg(test)]
pub(crate) use whole_file::{WholeFileOutcome, classify_whole_file};

#[cfg(test)]
pub(crate) use transaction::ParentDirectories;

#[cfg(test)]
mod compare_tests;

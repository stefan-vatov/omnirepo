//! Private lifecycle effects and durable run records.

#![allow(dead_code)]

mod adapters;
mod admission;
#[cfg(test)]
mod authority_integration_tests;

mod agent_confinement;
mod agent_framing;
mod agent_runtime;
mod cancellation;
mod commit_journal;
mod event;
mod fleet_permits;
mod invocation;
mod journal;
mod nested_permits;
mod push_reconcile;
mod remote_push;
mod remote_target;
mod replace;
mod replay;
mod run_record;
mod scheduler;
mod source_catalog;
mod source_extraction;
mod stages;
mod transaction_evidence;

pub(crate) use invocation::run_invocation;

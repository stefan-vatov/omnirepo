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
mod diagnostics;
mod event;
mod fleet_permits;
mod invocation;
mod journal;
mod nested_permits;
mod plan_builder;
mod plan_selection;
mod push_reconcile;
mod record_finalize;
mod remote_push;
mod remote_target;
mod replace;
mod replacement_requests;
mod replay;
mod run_record;
mod run_summary;
mod scheduler;
mod source_catalog;
mod source_extraction;
mod stages;
mod sync_plan;
mod terminal_projection;
mod transaction_evidence;

pub(crate) use invocation::run_invocation;

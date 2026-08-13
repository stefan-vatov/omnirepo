//! Private lifecycle effects and durable run records.

#![allow(dead_code)]

mod adapters;
mod admission;
mod agent_confinement;
mod agent_framing;
mod agent_runtime;
mod cancellation;
mod commit_journal;
mod event;
mod fleet_permits;
mod invocation;
mod journal;
mod remote_target;
mod replace;
mod replay;
mod run_record;
mod source_catalog;
mod stages;
mod transaction_evidence;

pub(crate) use invocation::run_invocation;

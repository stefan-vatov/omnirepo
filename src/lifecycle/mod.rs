//! Private lifecycle effects and durable run records.

#![allow(dead_code)]

mod adapters;
mod admission;
mod agent_framing;
mod event;
mod invocation;
mod journal;
mod replace;
mod replay;
mod run_record;
mod transaction_evidence;

pub(crate) use invocation::run_invocation;

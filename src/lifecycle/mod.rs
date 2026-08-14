//! Private lifecycle effects and durable run records.

#![allow(dead_code)]

mod adapters;
mod admission;
#[cfg(test)]
mod authority_integration_tests;

#[cfg(test)]
mod repair_fixture_tests;

#[cfg(test)]
mod fleet_fixture_tests;

#[cfg(test)]
mod verification_fixture_tests;

#[cfg(test)]
mod sync_idempotence_tests;

mod agent_confinement;
mod agent_framing;
mod agent_runtime;
mod cancellation;
mod check_runner;
mod command_spec;
mod commit_journal;
mod diagnostic_aggregation;
mod diagnostics;
mod event;
mod fleet_app;
mod fleet_collector;
mod fleet_fanout;
mod fleet_permits;
mod git_delivery;
mod hostile_fixtures;
mod initial_pass;
mod initial_sync;
mod invocation;
mod journal;
mod lifecycle_model;
mod migration_decision;
mod model_generation;
mod model_property_suite;
mod nested_permits;
mod plan_builder;
mod plan_selection;
mod preflight;
mod push_reconcile;
mod record_finalize;
mod remote_push;
mod remote_target;
mod repair_causation;
mod repair_classify;
mod repair_deliver;
mod repair_delta;
mod repair_execute;
mod repair_fallback;
mod repair_fold;
mod repair_reapply;
mod repair_reserve;
mod repair_selection;
mod repair_snapshot;
mod replace;
mod replacement_requests;
mod replay;
mod repository_preflight;
mod run_record;
mod run_summary;
mod scheduler;
mod single_repo_pass;
mod source_catalog;
mod source_extraction;
mod stages;
mod sync_plan;
mod terminal_projection;
mod transaction_evidence;
mod verification_gate;
mod verifier_confinement;
mod verify_and_gate;
mod work_mapping;

pub(crate) use invocation::run_invocation;

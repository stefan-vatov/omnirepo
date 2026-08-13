//! Hermetic doubles shared by Omnirepo's integration tests.
//!
//! The crate owns the fixture sources and has no dependency on the product
//! package. Consumers invoke the public product binary through these doubles.

pub mod agent_double;
pub mod cross_domain_fixture;
pub mod e2e_runner_crimson_coast;
pub mod failure_replay;
pub mod git_double;
pub mod lifecycle_fixture;
pub mod network_double;
pub mod process_double;
pub mod recovery_control;
pub mod test_evidence;

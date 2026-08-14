//! The direct-output prohibition guard.
//!
//! No worker or adapter writes stdout/stderr directly: operational facts
//! flow through typed journal events and the one renderer selected by
//! .27/.11.  The only allowed direct-output site is the CLI projection
//! boundary (invocation.rs), whose stderr diagnostics follow the decided
//! stream contract.  This module owns source cleanup and the
//! direct-output prohibition; renderer behavior tests and complete E2E
//! journeys live in their owner beads.

#![allow(dead_code)]

#[cfg(test)]
mod output_guard_tests;

/// One direct-output violation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectOutput {
    pub path: String,
    pub line: String,
}

/// The projection boundary: the only source allowed to write to
/// stdout/stderr directly (the CLI invocation seam).
pub fn is_projection_boundary(path: &str) -> bool {
    path.ends_with("src/lifecycle/invocation.rs")
}

/// Scan product sources for direct output calls.  Every
/// `println!`/`eprintln!`/`print!` outside the projection boundary is a
/// violation; raw evidence must stay in the record, never in the
/// streams.
pub fn assert_no_direct_output(sources: &[(String, String)]) -> Vec<DirectOutput> {
    let mut violations = Vec::new();
    for (path, content) in sources {
        if is_projection_boundary(path) || path.ends_with("output_guard.rs") {
            continue;
        }
        for (index, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.contains("println!(")
                || trimmed.contains("eprintln!(")
                || trimmed.contains("print!(")
            {
                // Test files are permitted (they capture their own
                // output); product workers are not.
                if path.contains("_tests") || path.ends_with("_tests.rs") {
                    continue;
                }
                violations.push(DirectOutput {
                    path: path.clone(),
                    line: format!("{}: {trimmed}", index + 1),
                });
            }
        }
    }
    violations
}

---
status: normative
scope: [repository-wide]
validation:
  - .github/workflows/pr-lint-test.yml
  - .github/workflows/coverage.yml
  - .github/workflows/pre-commit-hooks.yml
---

# Engineering standards

## Toolchain support

The minimum supported Rust version is 1.86; the complete target test suite
and documentation tests must pass on Rust 1.86 (the MSRV floor profile).

The primary quality gates run on Rust 1.95, the latest pinned toolchain:
CI and every local machine must run the same pinned version. Toolchain
resolution is repo-owned (`scripts/cargo-pinned`): rustup when present, the
direct rustup toolchain otherwise, and the system `cargo` when it already
reports the pinned version. A machine whose `cargo` does not resolve to the
pinned version is a configuration error, never a silent toolchain switch.

## Required validation

Every change must pass:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `cargo test --workspace --all-targets --all-features --locked`
- `cargo test --workspace --doc --all-features --locked`
- `cargo build --workspace --all-targets --all-features --locked`
- `prek run --all-files`

All Cargo validation commands that resolve dependencies must use `--locked`,
so validation fails rather than updating the committed lockfile.

## Tracker discipline

Beads are updated as they are worked. Every status change, evidence comment,
and dependency change is recorded through `br` at the moment the work
happens, never in a batch at the end. Tracker reads use `br` or the `bv`
robot commands (`--robot-next`, `--robot-triage`, `--robot-plan`); they never
depend on exported pages, cached viewer snapshots, or hand-edited copies of
tracker data. When a read and a live `br`/`bv` query disagree, the live query
wins and the stale source is discarded.

## Agent collaboration

Agents communicate exclusively through Agent Mail (`am`). Every message
between coordinating and worker agents is sent and acknowledged through
`am`; ad-hoc chat channels, unlogged file drops, and silent edits to shared
files without a reservation are not coordination. Overlapping file edits are
coordinated with `am file_reservations` before any write. Each worker's
outcome is delivered as an `am` handoff message, never as an unrecorded
side effect.

## Coverage

Coverage is measured across the workspace, all targets, and all features with
locked dependencies. Global coverage must remain at or above 80% of lines,
73% of functions, and 78% of regions. Lines added or modified by a change must
remain at or above 80% coverage.

Coverage percentages complement behavioral proof; they do not replace it.
Critical safety paths require direct tests of their authority boundaries and
failure behavior. Tests must not exist solely to execute trivial accessors or
private formatting for a higher percentage. Each behavior has one primary test
owner, and duplicate assertions belong with that owner rather than in parallel
coverage-only tests.

A threshold failure remains a failed check. Coverage reports may still be
generated and retained to diagnose the failure, but report generation must not
mask the gate result.

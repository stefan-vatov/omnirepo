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

The minimum supported Rust version is 1.86. The complete target test suite and
documentation tests must pass on Rust 1.86.

## Required validation

Every change must pass:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `cargo test --workspace --all-targets --all-features --locked`
- `cargo test --workspace --doc --all-features --locked`
- `cargo build --workspace --all-targets --all-features --locked`
- the repository's configured pre-commit hooks

All Cargo validation commands that resolve dependencies must use `--locked`,
so validation fails rather than updating the committed lockfile.

## Coverage

Coverage is measured across the workspace, all targets, and all features with
locked dependencies. Global coverage must remain at or above 90% of lines,
80% of functions, and 80% of regions. Lines added or modified by a change must
remain at or above 95% coverage.

Coverage percentages complement behavioral proof; they do not replace it.
Critical safety paths require direct tests of their authority boundaries and
failure behavior. Tests must not exist solely to execute trivial accessors or
private formatting for a higher percentage. Each behavior has one primary test
owner, and duplicate assertions belong with that owner rather than in parallel
coverage-only tests.

A threshold failure remains a failed check. Coverage reports may still be
generated and retained to diagnose the failure, but report generation must not
mask the gate result.

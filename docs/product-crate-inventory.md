---
status: reference
scope: [product-crate, package, public-api, workspace]
related: [../canon/architecture/product-distribution.md, ../canon/standards.md]
---

# Product crate inventory

This document records the current Cargo workspace and the contents of the
publishable product package. It is an evidence-based inventory, not a second
manifest and not an implementation plan. Removed source paths are not part of
the current product surface.

Checked on 2026-08-13 by AmberLynx while the worktree contained unrelated peer
changes. Evidence came from the current filesystem, `Cargo.toml`, locked Cargo
metadata, and the locked package listing.

## Workspace topology

The workspace has one publishable product package and two private packages.
The private packages are workspace members so developer tooling and reusable
fixtures can be tested with the product, but they are not product dependencies
and are not shipped.

| Package | Publish status | Targets and current paths | Direct dependencies | Product role |
| --- | --- | --- | --- | --- |
| `omnirepo` `0.8.3` | publishable (`publish` is not restricted) | binary `omnirepo` at `src/main.rs`; 20 root integration-test targets under `tests/` | runtime: `clap` with `derive`, `semver`; dev: `assert_cmd`, `cargo_metadata`, `predicates`, `serde` with `derive`, `syn`, `tempfile`, `toml_edit`, `yaml_serde` | The only product crate and the only shipped executable |
| `omnirepo-dev` `0.1.0` | `publish = false` | library `tools/omnirepo-dev/src/lib.rs`; thin binary `tools/omnirepo-dev/src/main.rs`; private tests in `tools/omnirepo-dev/tests/` | `serde` with `derive`, `serde_json` | Private Rust replacement for repository Beads tooling |
| `omnirepo-test-support` `0.0.0` | `publish = false` | library `tools/omnirepo-test-support/src/lib.rs`; private tests in `tools/omnirepo-test-support/tests/` | `tempfile` | Reusable process, network, Git, agent, lifecycle, and recovery fixtures |

The workspace is declared in the root `Cargo.toml` with resolver `3`. All
members use Rust edition `2024`, MSRV `1.95`, and the shared workspace lints.
`Cargo.lock` is tracked in Git and is used for every locked validation command.

## Product boundary and dependency direction

`src/main.rs` is the private composition root. It declares the product modules
and the CLI entry point; there is no `src/lib.rs`, no `src/bin/main.rs`, and no
`[lib]` target. `src/platform/authority/mod.rs` is the current private authority
module. The product runtime dependencies are `clap` and `semver`; it has no path
dependency to either private workspace crate.

The dependency direction is one way:

```text
omnirepo (publishable binary, runtime clap + semver)
        └── no dependency on private workspace crates

omnirepo-dev (private developer library + CLI, serde/serde_json)
omnirepo-test-support (private fixture library, tempfile)
```

The private crates also do not depend on the product. Integration contracts
prove the workspace boundary, package exclusions, dependency allowlist, and
test-support ownership. Tests may use the private crates as development
infrastructure; that does not widen the product runtime surface.

## Product source and test ownership

| Path | Current ownership | Published in the product package |
| --- | --- | --- |
| `src/main.rs` | Private binary composition root and CLI | yes |
| `src/configuration/mod.rs` | Private configuration context | yes |
| `src/lifecycle/mod.rs` | Private lifecycle context | yes |
| `src/lifecycle/run_record.rs` | Private lifecycle run-record implementation | yes |
| `src/managed_content/mod.rs` | Private managed-content context | yes |
| `src/managed_content/transaction.rs` | Private managed-content transaction implementation | yes |
| `src/platform/mod.rs` | Private platform context | yes |
| `src/platform/authority/mod.rs` | Private platform authority implementation | yes |
| `src/repository/mod.rs` | Private repository context | yes |
| `src/repository/policy.rs` | Private repository policy implementation | yes |
| `src/repository/state.rs` | Private repository state implementation | yes |
| `src/source/mod.rs` | Private source context | yes |
| `src/source/snapshot.rs` | Private source snapshot implementation | yes |
| `tests/` | Root integration contracts, fixtures, and quality checks | no |
| `tools/omnirepo-dev/` | Private developer tooling and its tests | no |
| `tools/omnirepo-test-support/` | Private reusable test-support code and its tests | no |
| `docs/` | Repository documentation, including this inventory | no |
| `scripts/` | Repository quality and coverage entry points | no |

The old shared library composition, legacy `clone`, `config`, `new`, `run`,
`sync`, and `util` product modules are absent from the current filesystem and
are not compatibility surfaces. Their prior paths must not be reintroduced as
aliases or re-exports.

## Exact `cargo package --list` classification

The current locked package listing contains exactly these entries:

```text
.cargo_vcs_info.json
CHANGELOG.md
CONSTITUTION.md
Cargo.lock
Cargo.toml
Cargo.toml.orig
LICENSE
README.md
src/configuration/mod.rs
src/lifecycle/mod.rs
src/lifecycle/run_record.rs
src/managed_content/mod.rs
src/managed_content/transaction.rs
src/main.rs
src/platform/authority/mod.rs
src/platform/mod.rs
src/repository/mod.rs
src/repository/policy.rs
src/repository/state.rs
src/source/mod.rs
src/source/snapshot.rs
```

### Runtime product entries

These are the intended package contents:

```text
CHANGELOG.md
CONSTITUTION.md
Cargo.lock
Cargo.toml
LICENSE
README.md
src/configuration/mod.rs
src/lifecycle/mod.rs
src/lifecycle/run_record.rs
src/managed_content/mod.rs
src/managed_content/transaction.rs
src/platform/authority/mod.rs
src/platform/mod.rs
src/repository/mod.rs
src/repository/policy.rs
src/repository/state.rs
src/source/mod.rs
src/source/snapshot.rs
src/main.rs
```

`Cargo.toml` explicitly excludes development-only paths and repository
metadata, including `.beads/`, `.cargo/`, `.github/`, `.codex/`, `.claude/`,
`canon/`, `docs/`, `scripts/`, `tests/`, `tools/`, and the local agent and
pre-commit files. The package listing contains none of those paths.

### Cargo-generated package metadata

`.cargo_vcs_info.json` and `Cargo.toml.orig` are generated by Cargo while it
stages a package. They are not present as current worktree source files, are
not runtime assets, and are not evidence of a second product manifest. Cargo
emits them in `cargo package --list` even though the root manifest excludes
the corresponding names. Inventory and package tests therefore classify both
as generated staging metadata, while the runtime allowlist above remains the
authoritative product surface.

## Reproducible evidence commands

Run these from the repository root after peer edits have quiesced:

```text
rtk cargo metadata --format-version 1 --locked
rtk cargo package --list --allow-dirty --locked
rtk cargo test --workspace --all-targets --all-features --locked
rtk cargo test --workspace --doc --all-features --locked
rtk cargo build --workspace --all-targets --all-features --locked
rtk git diff --check
```

The focused contract tests are `product_binary_crate_contract`,
`workspace_boundary_contract`, `test_support_crate_contract`,
`dependency_surface_contract`, and `workspace_tooling_integration`. They
verify that the binary-only product, private crate boundaries, package
allowlist, tracked lockfile, and one-way dependency surface remain aligned
with this inventory.

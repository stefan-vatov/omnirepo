# Repository quality command manifest

The canonical quality map is [`scripts/quality-manifest.json`](../scripts/quality-manifest.json).
It is a versioned data file. The array order is the execution order. A runner
must not infer commands from this document, Cargo aliases, CI YAML, or a local
tool installation.

Each gate has these fields:

- `id`: stable gate name.
- `kind`: `gate` for one required check.
- `toolchain`: the required Rust or system toolchain.
- `working_directory`: repository-relative working directory (`.`).
- `argv`: one executable and its arguments. These are not shell strings.
- `failure_identity`: stable identifier for result accounting.
- `authority`: Canon or repository path that owns the command.
- `owner`: Bead that owns the gate. Coverage remains owned by `.34`.

## Ordered gates

| Order | ID | Toolchain | Exact command | Failure identity | Authority |
| ---: | --- | --- | --- | --- | --- |
| 1 | `fmt` | stable | `cargo fmt --all -- --check` | `quality.fmt` | `canon/standards.md` |
| 2 | `clippy` | stable | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | `quality.clippy` | `canon/standards.md` |
| 3 | `tests` | stable | `cargo test --workspace --all-targets --all-features --locked` | `quality.tests` | `canon/standards.md` |
| 4 | `doctests` | stable | `cargo test --workspace --doc --all-features --locked` | `quality.doctests` | `canon/standards.md` |
| 5 | `build` | stable | `cargo build --workspace --all-targets --all-features --locked` | `quality.build` | `canon/standards.md` |
| 6 | `prek` | system | `prek run --all-files` | `quality.prek` | `canon/standards.md` |
| 7 | `beads-validate` | stable | `cargo run --quiet --locked --manifest-path tools/omnirepo-dev/Cargo.toml -- validate-decisions` | `quality.beads-validate` | `tools/omnirepo-dev/src/beads_validator.rs` |
| 8 | `beads-validator-tests` | stable | `cargo test --quiet --locked --manifest-path tools/omnirepo-dev/Cargo.toml --test beads_validator_contract` | `quality.beads-validator-tests` | `tools/omnirepo-dev/tests/beads_validator_contract.rs` |
| 9 | `beads-plan` | stable | `cargo run --quiet --locked --manifest-path tools/omnirepo-dev/Cargo.toml -- plan --repo-root . --json` | `quality.beads-plan` | `tools/omnirepo-dev/src/planner.rs` |
| 10 | `beads-plan-tests` | stable | `cargo test --quiet --locked --manifest-path tools/omnirepo-dev/Cargo.toml --test planner_contract` | `quality.beads-plan-tests` | `tools/omnirepo-dev/tests/planner_contract.rs` |
| 11 | `coverage` | Rust 1.86.0 and cargo-llvm-cov 0.8.7 | `bash scripts/coverage.sh` | `quality.coverage` | `.34` via `scripts/coverage.sh` |
| 12 | `msrv-tests` | Rust 1.86.0 | `cargo test --workspace --all-targets --all-features --locked` | `quality.msrv-tests` | `canon/standards.md` |
| 13 | `msrv-doctests` | Rust 1.86.0 | `cargo test --workspace --doc --all-features --locked` | `quality.msrv-doctests` | `canon/standards.md` |

The stable-toolchain gates and the MSRV gates have different identities even
when their Cargo arguments are the same. They prove different toolchain
contracts. The coverage gate is one delegation to `scripts/coverage.sh`; this
manifest does not repeat its thresholds, report commands, or `cargo-llvm-cov`
implementation.

## Execution profiles

The `profiles` array is part of the versioned manifest. Each profile has a
unique `name`, `kind: "profile"`, and a non-empty ordered list of existing
gate IDs. The required profiles are `full`, `stable`, `msrv`, and
`coverage`. The `full` profile contains every gate; it is also the default
when the `--profile` option is omitted. A profile selects gates, but the
runner always executes selected gates in the canonical order from `gates`, not
in profile array order.

Use the explicit profile option for a phase-specific run:

```text
omnirepo-dev quality --manifest PATH --repo-root PATH --profile stable --json
omnirepo-dev quality --manifest PATH --repo-root PATH --profile msrv --json
omnirepo-dev quality --manifest PATH --repo-root PATH --profile coverage --json
```

CI uses `stable` for the stable Rust and Beads checks, `msrv` for the Rust
1.86 checks, and `coverage` for the coverage-owned entry point. Local users can
run the same profile commands; no workflow-specific command list is required.

The runner validates all profile definitions before it resolves the repository
or starts a child process. Unknown profiles, duplicate names, empty profiles,
unknown or duplicate gate IDs, and missing required profiles fail with a
diagnostic. The selected profile name is recorded in the machine-readable
report.

## Aliases and nested checks

The Cargo aliases `fmt-check`, `lint`, `test-all`, `test-docs`, and `build-all`
are developer shortcuts. They map to the canonical gates in the `aliases`
array. They are not additional gates and must not receive new failure
identities. The prek gate may invoke hooks that also run Beads checks;
the direct Beads entries remain explicit workflow gates so their failures have
stable identities. This is a nested invocation, not a second command
authority.

## Lockfiles

`Cargo.lock` is tracked and is part of the package contract. Every Cargo gate
that can resolve dependencies carries `--locked`; validation fails instead of
rewriting the committed lockfile. The manifest records this as
`lockfiles.cargo_lock.update_behavior = "fail"`.

This repository has no Node package. `package-lock.json` is therefore
`not-used`; quality validation must not create or update it. This field records
the package-lock behavior without adding a Node toolchain or a second lockfile
authority.

## Consumer interface

The private `omnirepo-dev` quality command reads the JSON manifest and consumes
one gate at a time in array order. It receives `argv` as an argument array,
resolves `working_directory` below the repository root, and records
`failure_identity` with the process outcome. It preserves each gate's exit
status and does not replace a failed gate with a later diagnostic.

Any change to this interface must be coordinated through Agent Mail before the
private developer-tool crate is edited. The cutover command surface is owned by
the private crate and the repository wiring calls it directly.

## Aggregate runner execution contract

The repository-owned entry point is:

```text
omnirepo-dev quality --manifest PATH --repo-root PATH --json
```

The runner executes each `argv` array directly as an operating-system process.
It does not invoke a shell, expand aliases, rewrite arguments, install a
toolchain, or infer a replacement command. `toolchain` is required evidence
about the environment in which the gate is expected to run; the runner reports
that value and leaves toolchain selection to the command/environment already
provided by the caller.

For each gate, `working_directory` is resolved relative to `repo-root` and
must remain below that root. The child inherits the runner's environment
unchanged. The report records the resolved working directory and declared
toolchain so a run can be reproduced with the same caller environment.

The JSON report is the runner's only stdout. It contains one ordered result for
every manifest gate, including gates after a failure, with the gate ID,
`failure_identity`, declared toolchain, resolved working directory, process
exit status, success flag, and complete captured stdout and stderr. It also
records the selected `profile`. Runner
diagnostics that prevent a report are written to stderr. The runner exits zero
only when every gate succeeds; after all gates have run it exits nonzero if any
gate failed.

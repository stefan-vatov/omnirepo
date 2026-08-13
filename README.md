# Omnirepo

>This tool is currently in its early stages of development. You are welcome to use it if it appears beneficial for your needs; however, please be prepared to encounter rough spots.

Omnirepo is a command-line tool for managing multiple Git repositories.

## Table of Contents

- [Omnirepo](#omnirepo)
  - [Table of Contents](#table-of-contents)
  - [Installation](#installation)
  - [Testing and coverage](#testing-and-coverage)
    - [Quality checks](#quality-checks)
    - [Feature-test suite](#feature-test-suite)
    - [Coverage](#coverage)
  - [Usage](#usage)
    - [CLI Help](#cli-help)
  - [Contributing](#contributing)
  - [License](#license)

## Installation

1. Clone this repository.
2. Navigate to the project's root directory and run `cargo build --release --locked`.
3. Add the compiled binary to your `PATH`.

## Testing and coverage

The crate declares Rust 1.86 as its minimum supported toolchain. Install that
toolchain with the formatting and lint components before running the local
checks:

```sh
rustup toolchain install 1.86.0 --profile minimal
rustup component add rustfmt clippy --toolchain 1.86.0
```

### Quality checks

Local and CI quality checks use the repository-owned aggregate manifest. Run
the complete quality gate from the repository root:

```sh
cargo run --quiet --locked \
  --manifest-path tools/omnirepo-dev/Cargo.toml -- quality \
  --manifest scripts/quality-manifest.json --repo-root . --json
```

The runner executes every gate in manifest order and reports every failure. It
does not hide a failed gate behind a later command. The Cargo aliases remain
available as fast shortcuts for the five Rust-only gates:

```sh
cargo +1.86.0 fmt-check
cargo +1.86.0 lint
cargo +1.86.0 test-all
cargo +1.86.0 test-docs
cargo +1.86.0 build-all
```

All dependency-resolving commands use `--locked`, so local checks exercise the
same dependency graph as CI.

### Feature-test suite

Local and CI feature tests use one repository-owned orchestrator. It supports
case, suite, and full-matrix selection while preserving every worker outcome
and artifact:

```sh
cargo run --quiet --locked --manifest-path tools/omnirepo-dev/Cargo.toml -- \
  test --manifest scripts/test-suite-manifest.json --repo-root . \
  --full --jobs 1 --json
```

See [the test-suite reference](docs/testing/test-suite.md) for selection,
isolation, structured events, replay references, and quality delegation.

### Coverage

`cargo-llvm-cov` 0.8.7 currently needs Rust 1.87 or newer only when it is
installed from source. Install it with a newer toolchain, while continuing to
run coverage against this crate's Rust 1.86 toolchain:

```sh
rustup toolchain install 1.87.0 --profile minimal
cargo +1.87.0 install cargo-llvm-cov --version 0.8.7 --locked
rustup component add llvm-tools-preview --toolchain 1.86.0
```

Use the repository-owned coverage entry point for both local checks and CI:

```sh
cargo run --quiet --locked \
  --manifest-path tools/omnirepo-dev/Cargo.toml -- quality \
  --manifest scripts/quality-manifest.json --repo-root . \
  --profile coverage --json
```

The manifest-owned `coverage` profile selects this entry point. It uses Rust
1.86.0 and cargo-llvm-cov 0.8.7, measures the workspace with all targets and
features using locked dependencies, and enforces 90% global line coverage,
95% changed executable-line coverage, 80% function coverage, and 80% region
coverage. Critical safety boundaries and failure paths need direct tests;
trivial accessor or private-format padding is not a valid coverage strategy,
and each behavior has one primary test owner. It writes text, LCOV, and HTML
reports below the ignored `coverage/` directory. A threshold or report failure
remains the command failure after diagnostic reports are generated. GitHub
Actions invokes the same profile and uploads the reports for 14 days.

## Usage

### CLI Help

```plaintext
A tool for managing multiple git repositories

Usage: omnirepo

Options:
  -h, --help               Print help
  -V, --version            Print version
```

## Contributing

Contributions are welcome! Please submit a pull request or create an issue to propose changes or report bugs.

## License

This project is open source and available under the MIT License.

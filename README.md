# Omnirepo

>This tool is currently in its early stages of development. You are welcome to use it if it appears beneficial for your needs; however, please be prepared to encounter rough spots.

Omnirepo is a command-line tool for managing multiple Git repositories. It allows you to organize, clone, and run commands in multiple repositories simultaneously. Omnirepo is especially useful for developers who work with multiple repositories, making the workflow more efficient and streamlined.

## Table of Contents

- [Omnirepo](#omnirepo)
  - [Table of Contents](#table-of-contents)
  - [Features](#features)
  - [Installation](#installation)
  - [Testing and coverage](#testing-and-coverage)
    - [Quality checks](#quality-checks)
    - [Coverage](#coverage)
  - [Usage](#usage)
    - [CLI Help](#cli-help)
    - [Config File](#config-file)
  - [Commands](#commands)
    - [new](#new)
    - [clone](#clone)
    - [run](#run)
    - [sync](#sync)
  - [Contributing](#contributing)
  - [License](#license)

## Features

- Manage multiple Git repositories from a single config file.
- Clone repositories in parallel.
- Run commands in each repository simultaneously.
- Synchronize files across repositories. (TODO)

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

The repository provides Cargo aliases that mirror the CI quality gates. Run
them with the project toolchain:

```sh
cargo +1.86.0 fmt-check
cargo +1.86.0 lint
cargo +1.86.0 test-all
cargo +1.86.0 test-docs
cargo +1.86.0 build-all
```

All dependency-resolving commands use `--locked`, so local checks exercise the
same dependency graph as CI.

### Coverage

`cargo-llvm-cov` 0.8.7 currently needs Rust 1.87 or newer only when it is
installed from source. Install it with a newer toolchain, while continuing to
run coverage against this crate's Rust 1.86 toolchain:

```sh
rustup toolchain install 1.87.0 --profile minimal
cargo +1.87.0 install cargo-llvm-cov --version 0.8.7 --locked
rustup component add llvm-tools-preview --toolchain 1.86.0
```

The coverage aliases generate a gated summary or HTML output. A separate LCOV
command is shown below because its parent directory must already exist. The
coverage floors are 95% of lines and 80% of both functions and regions,
measured from production source only. Companion
`*_tests.rs` files and integration-test harnesses are excluded automatically by
`cargo-llvm-cov`. Reports are written below the ignored `coverage/` directory:

```sh
cargo +1.86.0 coverage
cargo +1.86.0 coverage-html
mkdir -p coverage
cargo +1.86.0 llvm-cov --workspace --all-targets --all-features --locked \
  --lcov --output-path coverage/lcov.info
```

For one test run that emits all report formats, use the same sequence as CI:

```sh
mkdir -p coverage
cargo +1.86.0 llvm-cov clean --workspace
cargo +1.86.0 llvm-cov --workspace --all-targets --all-features --locked \
  --no-report
cargo +1.86.0 llvm-cov report --summary-only --fail-under-lines 95 \
  --fail-under-functions 80 --fail-under-regions 80 \
  | tee coverage/summary.txt
cargo +1.86.0 llvm-cov report --lcov --output-path coverage/lcov.info
cargo +1.86.0 llvm-cov report --html --output-dir coverage
```

The GitHub Actions coverage job uses the stable Rust toolchain and a prebuilt
`cargo-llvm-cov` binary, then uploads the text, LCOV, and HTML reports as a
workflow artifact.

## Usage

### CLI Help

```plaintext
A tool for managing multiple git repositories

Usage: omnirepo [OPTIONS] <COMMAND>

Commands:
  new    Create a new repository
  clone  Clone a group of repositories based on tags
  run    Run a command in each repository
  sync   Sync a file across all repositories
  help   Print this message or the help of the given subcommand(s)

Options:
  -c, --config <CONFIG>    Point to a .omnirepo.yaml or a directory containing config
  -v, --verbose <VERBOSE>  Log to file [possible values: true, false]
  -h, --help               Print help
  -V, --version            Print version
```

### Config File

Create a `.omnirepo.yaml` file in your user's home directory with the following format (example):

Each template and included file has a unique `id`, which can be used when
selecting a template for synchronization.

```yml
---

repositories:
  - name: Glimmer config
    url: <valid-clone-url>
    dest: glimmer_config
    tags:
      - config
      - ansible
  - name: Private dotfiles
    url: <valid-clone-url>
    dest: dotfiles
    tags:
      - config
      - dotfiles

templates:
  - name: pre-commit
    id: pre-commit-v1
    url: https://raw.githubusercontent.com/stefan-vatov/omni-templates/main/default/.pre-commit-config.yaml
    kind: File
    dest: "."
    tags:
      - default
      - ci
  - name: .gitignore
    id: gitignore-v1
    url: https://raw.githubusercontent.com/stefan-vatov/omni-templates/main/default/.gitignore
    kind: File
    dest: "."
    tags:
      - default
  - name: GitHub Workflows
    id: github-workflows-v1
    url: https://raw.githubusercontent.com/stefan-vatov/omni-templates/main/github_workflows
    kind: Dir
    included_files:
      - file_name: pre-commit-hooks.yml
        id: pre-commit-hooks-v1
        dest: .github/workflows
    tags:
      - ci

```

## Commands

### new

Create a new repository.

Passing tags with `-t` for the new repo is optional.
Any files with the `default` tag _will be_ automatically added.


```plaintext
Create a new repository

Usage: omnirepo new [OPTIONS] --name <NAME>

Options:
  -n, --name <NAME>                The name of the repository
  -t, --tags <TAGS>                The names of the tags to clone
  -d, --destination <DESTINATION>  Destination to create new repository, current folder by default
  -h, --help                       Print help
```

### clone

Clone a group of repositories based on tags.

```plaintext
Clone a group of repositories based on tags

Usage: omnirepo clone [OPTIONS]

Options:
  -t, --tags <TAGS>                The names of the tags to clone
  -d, --destination <DESTINATION>  Destination to clone the repositories, current folder by default
  -h, --help                       Print help
```

### run

Run a command in each repository.

```plaintext
Run a command in each repository

Usage: omnirepo run [OPTIONS] --command <COMMAND>

Options:
  -c, --command <COMMAND>          The command to run
  -d, --destination <DESTINATION>  Destination to folder where the repos were cloned, current folder by default.
  -h, --help                       Print help
```

### sync

Sync a file across all repositories.
If the file does not exist it will be created.

```plaintext
Sync a file across all repositories

Usage: omnirepo sync [OPTIONS] --file <FILE>

Options:
  -f, --file <FILE>                    The file to sync
  -u, --url <URL>                      Source file for syncing from URL
  -s, --source-file <SOURCE_FILE>      Local source file for syncing
  -t, --template-file <TEMPLATE_FILE>  Configured template ID for syncing
  -d, --destination <DESTINATION>      Destination to folder where the repos were cloned, current folder by default.
  -h, --help                           Print help
```

## Contributing

Contributions are welcome! Please submit a pull request or create an issue to propose changes or report bugs.

## License

This project is open source and available under the MIT License.

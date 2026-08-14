# Omnirepo

Omnirepo is a constitutional synchronization tool: it converges managed
files and sections from ordered source repositories into declared
destination repositories, unattended, byte-exactly.

## Commands

The public command surface is exactly `sync`, `setup`, and `validate`.
Humans and agents operate the same surface; routine success is quiet.

| Command | Purpose |
|---------|---------|
| `sync` | Synchronize managed files and sections from the machine-declared sources into the selected destinations. |
| `setup` | Author the canonical machine configuration (interactive confirmation or `--yes` non-interactively). |
| `validate` | Validate machine configuration and repository policy without effects. |

There is no `migrate` command and no general repository orchestration
surface in the first constitutional release.

## First unattended sync

1. Install the binary (see below).
2. Author the canonical machine configuration:

   ```yaml
   # <HOME>/.omnirepo/config.yaml
   version: 1
   repositories:
     - id: destination-a
       path: /srv/repositories/a
   sources:
     - id: upstream
       location: https://example.com/repo.git
   concurrency:
     max_repositories: 4
     max_child_work: 8
   ```

   `setup` authors this file for you; applying the same intent repeatedly
   is a no-op and an invalid or conflicting authority is never replaced.

3. Run the first synchronization:

   ```sh
   omnirepo sync
   ```

   The run creates a durable record at
   `<HOME>/.omnirepo/runs/<timestamp>-<id>.log`, synchronizes the
   managed content, runs the declared verification, and delivers the
   scoped commit. Exit codes are stable: `0` success (including
   unchanged and empty fleets), `2` invocation or configuration
   failure, `3` partial fleet failure, `4` every selected repository
   failed, `5` durable-record failure, `130` user cancellation.

See [the quickstart](docs/quickstart.md) for a complete walkthrough and
[docs/breaking-guidance.md](docs/breaking-guidance.md) for what changed
from earlier releases.

## Installation

Build from source with the pinned toolchain:

```sh
cargo build --release --locked
```

The binary is `target/release/omnirepo`. The first constitutional
release supports Linux and macOS on ordinary local filesystems.

## Testing and coverage

Local and CI quality checks use the repository-owned aggregate manifest.
Run the complete quality gate from the repository root:

```sh
cargo run --quiet --locked \
  --manifest-path tools/omnirepo-dev/Cargo.toml -- quality \
  --manifest scripts/quality-manifest.json --repo-root . --json
```

The runner executes every gate in manifest order and reports every
failure. The Cargo aliases remain available as fast shortcuts for the
five Rust-only gates:

```sh
cargo +1.86.0 fmt-check
cargo +1.86.0 lint
cargo +1.86.0 test-all
cargo +1.86.0 test-docs
cargo +1.86.0 build-all
```

All dependency-resolving commands use `--locked`, so local checks
exercise the same dependency graph as CI.

### Feature-test suite

Local and CI feature tests use one repository-owned orchestrator:

```sh
cargo run --quiet --locked --manifest-path tools/omnirepo-dev/Cargo.toml -- \
  test --manifest scripts/test-suite-manifest.json --repo-root . \
  --full --jobs 1 --json
```

### Coverage

Use the repository-owned coverage entry point:

```sh
cargo run --quiet --locked \
  --manifest-path tools/omnirepo-dev/Cargo.toml -- quality \
  --manifest scripts/quality-manifest.json --repo-root . \
  --profile coverage --json
```

The manifest-owned `coverage` profile uses Rust 1.86.0 and cargo-llvm-cov
0.8.7, enforces the configured thresholds, and writes text, LCOV, HTML,
ownership, and changed-line reports below the ignored `coverage/`
directory. The changed executable-line gate compares the current `HEAD`
against one explicit base revision supplied through
`OMNIREPO_COVERAGE_BASE`.

## Contributing

Contributions are welcome! Please submit a pull request or create an
issue to propose changes or report bugs.

## License

See the repository license file.

# Omnirepo

Omnirepo is a constitutional synchronization tool: it converges managed
files and sections from ordered source repositories into declared
destination repositories, unattended, byte-exactly.

## Commands

The public command surface is exactly `sync`, `setup`, and `doctor`.
Humans and agents operate the same surface; routine success is quiet.

| Command | Purpose |
|---------|---------|
| `sync` | Synchronize managed files and sections from the machine-declared sources into the selected destinations. |
| `setup` | Author the canonical machine configuration (interactive confirmation or `--yes` non-interactively). |
| `doctor` | Diagnose the machine without effects: configuration, source availability, declarations, and cross-source conflicts, with every shadowed item and its winner named. |

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

See [the quickstart](docs/quickstart.md) for a complete walkthrough, [docs/breaks-inventory.md](docs/breaks-inventory.md) for what changed from earlier releases, and [docs/breaking-guidance.md](docs/breaking-guidance.md) for the actionable migration guidance.

## Managed partial sections

Whole files sync byte-exactly. A **partial** manages one named region of
a destination file and leaves every other byte alone. One destination
file can hold many partials, each from a different source file — even
from different source repositories. This is how one `AGENTS.md` carries
shared rule blocks from several upstream repos next to purely local
content.

### How a partial works

- The source side is an ordinary file: the **whole source file** is the
  section body. No markers in the source.
- The declaration names the section with a stable ID:

  ```text
  omnirepo-declarations-v1
  source=platform revision=<pinned-sha> path=partials/rust-rules.md id=agents-rust mode=section destination=AGENTS.md section=rust-rules
  ```

- On the destination side, `sync` owns the block between the named
  markers and writes them itself — you never hand-edit them:

  ```markdown
  # Local title

  Local notes stay local.

  <!-- omnirepo:start rust-rules -->
  Use the pinned toolchain.
  <!-- omnirepo:end rust-rules -->

  <!-- omnirepo:start security-rules -->
  Never log credentials.
  <!-- omnirepo:end security-rules -->
  ```

  When a named section is absent, `sync` appends it (markers included)
  after the existing content. When it exists, `sync` replaces only its
  body. Everything outside managed sections is preserved verbatim,
  byte-exact.

### Multiple sources, one file

Each source repository declares its own sections in its
`.omnirepo/source.yaml`. Distinct section IDs on one destination file
are independent: all of them land. When two sources claim the **same**
section ID, the machine configuration's source order decides the winner,
and the loser is reported — never silently merged. A whole-file claim
and a section claim on the same destination are incompatible and fail
before anything is written. Section IDs use ASCII lowercase letters,
digits, dots, underscores, and hyphens.

### Marker syntax by format

Markers are exact full lines in the destination format's comment syntax:
`<comment-token> omnirepo:start <section-id>` and the matching
`omnirepo:end` line. The registry resolves the destination path's
extension (last component, lowercase). Unknown or extensionless targets
fail closed with a typed error — a partial is never silently inferred:

| Destination format | Extensions | Example open marker |
|---|---|---|
| YAML / TOML / Shell | `.yml`, `.yaml`, `.toml`, `.sh`, `.bash` | `# omnirepo:start rust-rules` |
| Python / Ruby | `.py`, `.rb` | `# omnirepo:start rust-rules` |
| JSON / JavaScript / TypeScript | `.json`, `.js`, `.mjs`, `.cjs`, `.ts`, `.mts`, `.cts` | `// omnirepo:start rust-rules` |
| Rust | `.rs` | `// omnirepo:start rust-rules` |
| Markdown / HTML | `.md`, `.markdown`, `.html`, `.htm` | `<!-- omnirepo:start rust-rules -->` |
| INI | `.ini` | `; omnirepo:start rust-rules` |
| SQL | `.sql` | `-- omnirepo:start rust-rules` |

Marker recognition is exact: unnamed, whitespace-altered, nested,
interleaved, unpaired, or duplicate-ID markers are typed failures, never
guesses, and the file is left unchanged. A payload line that looks like
a marker is invalid, never escaped. The full delimiter reference lives
in [docs/reference.md](docs/reference.md).

### Checking a machine

`omnirepo doctor` runs the same effect-free planning as `sync` and
reports the findings: every source's availability, every managed item
and the section it owns, every shadowed loser with its winner, and any
declaration that would fail at sync time (for example a section that
targets a format without a registered comment syntax). Exit `0` means
healthy; exit `2` means problems were found.

## Installation

Build from source with the pinned toolchain:

```sh
cargo build --release --locked
```

The binary is `target/release/omnirepo`. Omnirepo supports every Linux
filesystem and supports macOS on APFS.

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
cargo fmt-check
cargo lint
cargo test-all
cargo test-docs
cargo build-all
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

The manifest-owned `coverage` profile uses Rust 1.95.0 and cargo-llvm-cov
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

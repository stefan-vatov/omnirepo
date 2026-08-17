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

See [the quickstart](docs/quickstart.md) for a complete walkthrough, [docs/breaks-inventory.md](docs/breaks-inventory.md) for what changed from earlier releases, and [docs/breaking-guidance.md](docs/breaking-guidance.md) for the actionable migration guidance.

## Managed partial sections

Whole files sync byte-exactly. For a **partial**, mark the managed region
in both the source and the destination with the file format's comment
delimiters: `sync` replaces only the bytes between the markers and keeps
everything outside them verbatim. The markers are the format's canonical
comment syntax (`#`, `//`, or `<!-- -->`), so the file stays ordinary
source.

### Markdown destination

Insert the marker pair where the partial should live, for example in
`docs/site/README.md`:

```markdown
<!-- omnirepo-start -->
# Monthly release highlights
- Roadmap for the next milestone
<!-- omnirepo-end -->
```

The source file carries the exact replacement content between the same
pair, and the declaration selects it with `mode=section`:

```text
omnirepo-declarations-v1
source=docs revision=<pinned-sha> path=highlights.md id=release-highlights mode=section destination=docs/site/README.md
```

Every `sync` rewrites only the block between the two Markdown comments;
the rest of `README.md` is preserved untouched.

### JavaScript / TypeScript destination

Same idea in `src/version.ts`:

```ts
// omnirepo-start
export const API_VERSION = process.env.API_VERSION ?? "unstable";
// omnirepo-end
```

### The top ten format families

The registry resolves the destination path's extension (last component,
lowercase) to exactly one marker pair. Unknown or extensionless targets
fail closed with a typed error — a partial is never silently inferred:

| Destination format | Extensions | Open marker | Close marker |
|---|---|---|---|
| YAML | `.yml`, `.yaml` | `# omnirepo-start` | `# omnirepo-end` |
| TOML | `.toml` | `# omnirepo-start` | `# omnirepo-end` |
| Shell | `.sh`, `.bash` | `# omnirepo-start` | `# omnirepo-end` |
| JSON | `.json` | `// omnirepo-start` | `// omnirepo-end` |
| JavaScript | `.js`, `.mjs`, `.cjs` | `// omnirepo-start` | `// omnirepo-end` |
| TypeScript | `.ts`, `.mts`, `.cts` | `// omnirepo-start` | `// omnirepo-end` |
| Markdown | `.md`, `.markdown` | `<!-- omnirepo-start -->` | `<!-- omnirepo-end -->` |
| HTML | `.html`, `.htm` | `<!-- omnirepo-start -->` | `<!-- omnirepo-end -->` |
| Python | `.py` | `# omnirepo-start` | `# omnirepo-end` |
| Rust | `.rs` | `// omnirepo-start` | `// omnirepo-end` |

Exactly one ordered, non-nested marker pair is required. Unpaired,
nested, multiple, or reversed markers are typed failures, never guesses.
The full delimiter reference lives in
[docs/reference.md](docs/reference.md).

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

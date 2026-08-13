---
status: reference
scope: [testing, tooling]
validation: [tools/omnirepo-dev/tests/test_suite_contract.rs]
---

# Repository test-suite command

`omnirepo-dev test` is the one local and CI entry point for the repository's
unit, component, end-to-end, adversarial, and platform case matrix. The
command is private repository tooling. It does not change the public
`omnirepo` command surface.

## Invocation

Run the complete matrix from the repository root:

```sh
cargo run --quiet --locked --manifest-path tools/omnirepo-dev/Cargo.toml -- \
  test --manifest scripts/test-suite-manifest.json --repo-root . --full \
  --jobs 1 --json
```

Use one case or one suite when iterating:

```sh
cargo run --quiet --locked --manifest-path tools/omnirepo-dev/Cargo.toml -- \
  test --manifest scripts/test-suite-manifest.json --repo-root . \
  --case canonical-acceptance-journeys --json

cargo run --quiet --locked --manifest-path tools/omnirepo-dev/Cargo.toml -- \
  test --manifest scripts/test-suite-manifest.json --repo-root . \
  --suite e2e --jobs 2 --json
```

`--case`, `--suite`, and `--full` are mutually exclusive. Without a selector,
the command uses the complete manifest. `--jobs` sets a fixed worker count;
completion order does not change the manifest order in the report or event
log. The first failed selected case in manifest order supplies the suite exit
status. Later cases still run and retain their own outcomes.

## Case outcomes and isolation

Every selected case receives a terminal result. A non-zero worker exit remains
`failed` with its original exit code. A missing executable is
`missing_tool` with status 127. An unavailable declared capability is
`unsupported_capability` with status 125. A timeout is `timed_out` with status
124. These outcomes are failures for the aggregate result; an unsupported
platform case is visible and is not reported as a pass.

Workers receive a fresh case root for `HOME`, temporary files, and test
artifacts. Standard input is closed. Standard output and standard error are
captured, bounded, sanitized, and written below the run artifact root. Worker
channels are not copied to the command's terminal output. The manifest may add
explicit environment values, but it cannot replace the isolated directories.

## Evidence and replay

The runner delegates structured event aggregation and safe artifact writes to
`omnirepo-test-support`. It emits the shared `omnirepo.test-event.v1` JSONL
schema at `<run-id>/events.jsonl`, plus a versioned aggregate report at
`<run-id>/report.json`. Each case report contains paths to its captured channels
and a replay reference. Full replay policy remains owned by the shared failure
replay module; this command only records the invocation identity and artifact
pointer.

The artifact root defaults to `target/omnirepo-test-artifacts`. Set
`--artifacts PATH` to use another authority root. Runs use unique IDs, so a
later invocation does not overwrite an earlier report.

## Quality delegation

The test manifest can point at `scripts/quality-manifest.json`. Supplying
`--quality-profile NAME` runs the existing repository quality authority and
stores its unchanged report under the suite artifacts. The test orchestrator
does not copy quality-gate policy or translate the delegated quality result.

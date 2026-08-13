# Coverage ownership

The coverage threshold remains owned by the repository coverage gate:
`scripts/coverage.sh` measures the locked workspace with all targets and all
features and enforces the Canon thresholds of 90% global lines, 95% changed
executable lines, 80% functions, and 80% regions. Critical safety boundaries
and their failure paths still require direct tests. Tests that exist only to
exercise trivial accessors or private formatting are not valid coverage, and
each behavior has one primary test owner. This page documents the separate
`.74.6` attribution step. It does not choose a threshold and it does not
replace the canonical test taxonomy.

## Canonical projection

`tests/traceability/matrix.json` remains the only test taxonomy and ownership
source. `tests/traceability/coverage-ownership.json` is a checked-in projection
that maps each publishable product source path to exactly one existing matrix
row. Each entry contains only the path and canonical row ID. The attribution
tool resolves the row's case, evidence, and primary-owner identity from the
matrix at run time, so drift fails closed instead of silently attributing a gap
to stale copied metadata.

The projection is intentionally file-level. It identifies the owning matrix
row for every uncovered line, function, and region in that source file. A file
that contains more than one behavior must receive a future range-aware mapping
change coordinated with the matrix owner; this tool never guesses from symbol
names or source text.

Development-only workspace crates are outside this product attribution scope.
They remain part of the normative workspace threshold. Runtime source files
under `src/` must be mapped exactly; missing, extra, duplicate, symlinked, or
ambiguous paths fail closed. Test-only files named `tests.rs`, ending in
`_tests.rs`, or below a directory whose exact path component is `tests` are
not product source. The directory rule matches the exact component `tests`;
names that merely contain `tests` are not excluded.

## Deterministic command

The private developer tool consumes the generated LCOV report and emits one
bounded JSON report:

```text
cargo run --quiet --locked --manifest-path tools/omnirepo-dev/Cargo.toml -- \
  coverage-ownership \
  --repo-root . \
  --lcov coverage/lcov.info \
  --matrix tests/traceability/matrix.json \
  --ownership tests/traceability/coverage-ownership.json \
  --json
```

The report gives exact line, function, and LCOV branch/region locations and
the corresponding matrix row and primary test Bead. It validates strict JSON
identity, rejects duplicate keys, rejects malformed or truncated LCOV, bounds
input and retained report size, and never reads or rewrites generated reports
as authority. The coverage gate must preserve its threshold exit status while
running this diagnostic step; successful report generation cannot turn a failed
threshold into success.

Every new or moved product source file requires an updated projection entry in
the same change. The focused contract is:

```text
cargo test --locked --manifest-path tools/omnirepo-dev/Cargo.toml \
  --test coverage_contract -- --nocapture
```

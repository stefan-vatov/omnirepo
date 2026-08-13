# Decision-aware Beads Viewer export

The private `omnirepo-dev` tool refreshes a deterministic, checked projection
from a versioned Viewer export. It is a regression and inspection boundary. It
does not claim Beads work, close owner decisions, or write server state.

## Refresh a projection

Run the repository-owned refresh command from the repository root:

```text
cargo run --locked --manifest-path tools/omnirepo-dev/Cargo.toml -- \
  viewer refresh --input tests/fixtures/viewer_export.json --json
```

The command writes one JSON projection to standard output. Redirect it to a
temporary file when inspection needs a file. Do not commit generated output or
server state. The command fails with no partial standard output when the input
cannot be read, cannot be parsed, has the wrong schema, or fails the checked
Viewer contract.

The regression contract runs the same command seam and checks:

- graph and list contain the same unique, sorted issue IDs;
- every declared status category, wording, filter, and count remains stable;
- detail rows agree with list badges and actionable flags;
- owner-decision, invalid, and stale rows remain non-actionable;
- the actionable IDs equal the checked canonical sources;
- raw `bv` recommendations remain advisory evidence and cannot promote work;
- repeated refreshes produce byte-identical JSON.

Run the focused contract with locked dependencies:

```text
cargo test -p omnirepo-dev --test viewer_export_regression_contract --locked
```

## Export and serve the Beads Viewer pages

The installed `bv` binary owns static page generation and preview serving. Use
its export and preview commands when you need a browser view:

```text
bv --export-pages /tmp/omnirepo-bv-pages
bv --preview-pages /tmp/omnirepo-bv-pages
```

The temporary directory is disposable. Do not add it to Git or treat its
contents as an authority. The Viewer is a display surface; the refresh command
and its checks are the repository-owned evidence boundary.

## Planning authority

Raw `bv --robot-plan`, `bv --robot-triage`, and similar recommendation output is
advisory only. It may help a human inspect the graph, but it cannot make an
agent claim work or make an owner decision.

An issue is actionable only when the checked planner has validated the
canonical sources and their agreement:

1. `br ready --json` supplies the Beads-ready set.
2. The checked autonomous planner validates tracked status and labels and
   supplies `omnirepo.checked-agent-plan.v1`.
3. The Viewer may display the intersection of those checked sources.

Owner decisions need the owner response. They remain visible in graph, list,
detail, and filters, but never enter the actionable queue. Closed decisions,
reopened or invalid decision state, and stale exports fail closed as display
rows rather than becoming work.

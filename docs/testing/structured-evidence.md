# Structured test evidence

The omnirepo-test-support crate is the evidence boundary for unit, component,
and end-to-end test runners. Worker code submits typed events to one
EventRecorder; it does not write terminal output. The recorder keeps a
correlated start and terminal event for every step.

## Event contract

Every event uses schema omnirepo.test-event.v1 and carries the same stable
identity:

- case_id, suite, repository, and stage;
- flattened source, plan, and config identity;
- attempt, seed, and command classification;
- event kind, correlation/event IDs, outcome, duration, artifact pointer, and
  optional sanitized diagnostic.

Terminal outcomes include passed, failed, skipped, and harness_failure. A
skipped capability and a worker that exits early are still terminalized. An
RAII step guard emits a harness terminal event when a worker forgets to finish.
Explicit peer registration lets finalization account for a worker that never
emits a start.

## Deterministic bundles

EventRecorder::finalize sorts events by the complete identity, then start before
terminal. It computes peer accounting and a quiet terminal projection. Parallel
completion order therefore cannot change JSONL bytes or the order of failure
evidence. EvidenceBundle::to_jsonl writes one JSON object per event followed by
exactly one summary record. from_jsonl rejects blank records, unknown fields,
non-final or duplicate summaries, and empty bundles. It validates event IDs,
correlations, identity, ordering, event outcomes, and start/terminal parity.
The summary accounting and projection are recomputed from the validated events
and must match exactly; persisted JSON cannot supply a second source of truth.

All public Deserialize implementations use strict wire types and rerun the
same constructor invariants. This includes identity fields, artifact and replay
references, event shape, accounting, projection, and bundle validation. Peer
accounting accepts only terminal outcomes and requires ordered, duplicate-free
terminal and missing partitions of the ordered expected case IDs. The
projection outcome and counts must agree, and missing peers must be represented
by harness failures. A correlated start and terminal event must retain the same
artifact and replay reference.

The projection includes counts and safe artifact/replay pointers. It does not
include raw diagnostics. TerminalProjection::render_quiet is the only
terminal-facing representation and remains concise.

## Evidence security and bounds

Diagnostics are sanitized before persistence. The redactor removes configured
secret values, URI userinfo, common authentication key/value values, and
terminal control sequences. Control bytes are escaped or replaced with a
visible marker. Process captures use the frozen
`sanitize_channels(&DiagnosticRedactor, stdout, stderr, max_bytes)` seam. It
lossily decodes both complete byte channels, applies the same sanitizer, then
allocates one stdout-then-stderr byte budget. `max_bytes` must be at least the
length of `DIAGNOSTIC_TRUNCATION_MARKER` and no greater than
`MAX_EVIDENCE_BYTES`. `SanitizedChannels::combined_bytes` includes one marker
when truncation occurs and is always within that budget; the first channel
whose text is cut receives the marker. Each `SanitizedDiagnostic` reports
redaction, control escaping, truncation, and invalid UTF-8 flags. Combined
diagnostics are therefore bounded to one MiB and retain an explicit
truncation marker without per-runner cap or redaction policy.

The persisted bound uses one byte accounting rule: count the UTF-8 bytes in
every persisted `TestEvent.diagnostic` string, once, in event order. The sum
must not exceed `MAX_EVIDENCE_BYTES`; an individual diagnostic cannot exceed the
same bound. Event identities, artifact pointers, projection fields, and JSON
syntax are not diagnostic bytes, and the summary has no diagnostic field. Both
direct serde deserialization and JSONL replay reject an oversized diagnostic or
an oversized combined bundle. No alternate unbounded diagnostic representation
is accepted.

Artifact pointers are relative paths only. ArtifactStore rejects absolute or
parent-traversing paths, control characters, symlink components in the complete
root ancestor chain, non-directory ancestors, and writes outside its authority
root. The root is canonicalized before use. On supported Unix platforms,
artifact writes walk directory file descriptors with no-follow flags and create
the final file exclusively, so an ancestor swap cannot redirect a write. A
replay pointer cannot silently overwrite an existing bundle.

DiagnosticRedactor and CaseExecution implement redacted Debug output. Secrets,
raw body diagnostics, and raw cleanup diagnostics never appear in public
debug formatting.

## Cleanup and replay

execute_case catches body failures, always runs the cleanup closure, and records
cleanup failures as harness failures. Its returned status is structured
evidence; it does not print worker diagnostics. The artifact pointer and
deterministic replay ID are retained in terminal events and the quiet
projection.

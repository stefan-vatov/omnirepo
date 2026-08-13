# Failure replay bundles

The omnirepo-test-support::failure_replay module is the failure boundary for
unit, component, and clean-environment E2E cases. It does not run a process or
contact a service. A runner supplies explicit fixture facts, then either writes
one bounded JSON bundle or receives a typed error. A bundle-write error remains
a suite failure; it must never turn a failed case green.

## Bundle contents

FailureReplayBundle uses schema omnirepo.failure-replay.v1. Its required
identity is:

- test manifest version;
- case ID, fixture ID, failure scenario, failure class, and deterministic seed;
- selected platform/filesystem contract and every requested capability result;
- sanitized command/config summary and a relative event-log path;
- an ordered barrier schedule;
- the first failing assertion and all durable events before that assertion;
- expected-versus-observed effect differences;
- terminal peer outcomes and cleanup failures;
- a replay disposition and the existing structured EvidenceBundle.

Failure classes are harness_failure, product_failure, and
unsupported_capability. The scenario enum covers process crashes, concurrent
runs, interrupted journals, ambiguous Git delivery, repair attempts, and
partial source availability.

## Replay and non-replayability

A replayable bundle stores argv, not an unescaped shell fragment. ReplayCommand
quotes every argument and gives the operator one command to run. The runner
must execute that command with its hermetic LifecycleFixture roots and then
submit a ReplayObservation to verify_replay. Verification compares case and
fixture identity, failure scenario and class, seed, selected platform
contract, barrier order, first assertion, and the preceding durable event
sequence. It returns Reproduced or a typed Diverged reason; it has no external
effect.

When deterministic replay is not lawful, the runner uses
NonReplayableReason. Reasons include missing seed or event log,
nondeterministic input, unsupported platform, external service or ambient state
requirements, corrupt evidence, and bundle-creation failure. A non-replayable
bundle has no command and remains visible as a failure.

## Bounds and security

The complete serialized bundle is at most MAX_FAILURE_REPLAY_BYTES, currently
the same one-MiB bound as structured test evidence. Individual metadata fields
are bounded to 4096 bytes and collections are bounded. Command/config values,
assertions, durable details, effects, peer diagnostics, cleanup diagnostics,
replay arguments, and every retained public EvidenceBundle field pass through
DiagnosticRedactor before persistence. Evidence events are rebuilt through the
public EventRecorder, which recomputes event IDs, peer accounting, and the
terminal projection while applying the same bound and control-sequence rules.
The complete diagnostic stream is sanitized as one deterministic sequence, so
a secret split across event or output chunks cannot bypass redaction.
Relative paths reject absolute, parent-traversing, empty, and control-character
components. Artifact persistence uses the no-follow, exclusive ArtifactStore
boundary, so a replay bundle cannot overwrite a previous bundle or escape its
fixture root.

Callers provide selected config values only. The module never reads
environment variables, user HOME, current time, repository state, network
services, credentials, or installed agents. Use logical fixture-root paths in
the replay recipe.

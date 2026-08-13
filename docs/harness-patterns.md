---
status: reference
scope: [lifecycle-fixtures, lifecycle-faults]
validation:
  - tests/harness_lint.rs
  - tools/omnirepo-test-support/tests/harness_usage_patterns.rs
---

# Hermetic lifecycle harness patterns

This page defines the `lifecycle-harness-usage/v1` composition contract for the
primitive fixture layer. The executable companion is
`tools/omnirepo-test-support/tests/harness_usage_patterns.rs`, executed as the
private `omnirepo-test-support` package test target. Keep each test responsible
for one boundary and one evidence bundle.

## Select and identify a fixture

Create one `LifecycleFixture` for each test. Give it a stable case ID and a
numeric seed. The pair is the seed/replay identity. Use only the roots returned
by `fixture.roots()` for HOME, machine configuration, source, destination, run
records, artifacts, and local remote data. Apply `fixture.environment()` to
every child command. This prevents a test from reading real HOME, machine
configuration, credentials, or a peer test's files.

Arm the named fault before the operation under test. Arm a named barrier before
starting a child or worker. A worker reports the barrier, the test waits for the
report, the test releases the barrier, and the test joins or reaps the worker.
The event log and an artifact under the fixture's `artifacts` root are the
observable evidence. Clean up only after every child has been reaped.

Capability checks are part of the fixture contract. Call `fixture.require(...)`
before using Git, aliases, FIFOs, or Unix permissions. A fixture may skip only
for the returned `Unsupported` capability; an I/O or protocol error remains a
test failure.

Do not use wall-clock sleeps, polling loops, or retries as synchronization. Use
the named barrier, deterministic clock, process marker, child wait, or thread
join that represents the state under test. The lint check extracts every Rust
code block below and rejects known wall-clock sleep calls.

## Component

- Fixture owner: `component-owner`
- Fault point: `component.read:before-read`
- Barrier: `component-ready`
- Seed/replay ID: `component-pattern/6101`
- Capability check: `fixture.require(Capability::UnixPermissions)`; skip only on `Unsupported`
- Evidence bundle path: `<fixture roots>/artifacts/component-pattern.evidence`

```rust
let mut fixture = LifecycleFixture::create(FixtureSpec::new("component-pattern", 6101))?;
fixture.require(Capability::UnixPermissions)?;
let fault = FaultPoint::new("component.read", "before-read", "component-owner", 1);
fixture.faults().arm(fault.clone(), FaultAction::ReturnError("injected-read-error".into()))?;
assert!(fixture.faults().hit(&fault).is_some());
fixture.faults().assert_consumed()?;

let barriers = fixture.barriers();
let gate = barriers.arm("component-ready")?;
let worker = std::thread::spawn(move || gate.hit());
barriers.wait_for_hit("component-ready")?;
barriers.release("component-ready")?;
worker.join().expect("component worker should join")?;

let evidence = fixture.roots().artifacts().join("component-pattern.evidence");
fs::write(&evidence, "owner=component-owner\nfault=component.read:before-read\nseed=6101\n")?;
fixture.track_ephemeral(evidence)?;
```

The component pattern proves that one named fault is consumed and that a
component cannot pass a barrier before the test releases it.

## Process tree

- Fixture owner: `process-tree`
- Fault point: `process.fork-late-write`
- Barrier: `barrier-hit` then explicit `release`
- Seed/replay ID: `process-tree-pattern/6202`
- Capability check: `fixture.require(Capability::UnixPermissions)`; skip only on `Unsupported`
- Evidence bundle path: `<fixture roots>/artifacts/process-tree-pattern.evidence`

```rust
let mut fixture = LifecycleFixture::create(FixtureSpec::new("process-tree-pattern", 6202))?;
fixture.require(Capability::UnixPermissions)?;
let mut process = FakeExecutable::spawn(
    &mut fixture,
    ProcessSpec::new("process-tree-pattern", ProcessBehavior::ForkAndLateWrite),
)?;
process.wait_for_barrier()?;
process.release()?;
let result = process.wait()?;
assert!(result.status.success());
assert!(result.evidence.late_write);
assert!(result.evidence.ambient_credentials_absent);

let evidence = fixture.roots().artifacts().join("process-tree-pattern.evidence");
fs::write(&evidence, "owner=process-tree\nfault=process.fork-late-write\nseed=6202\n")?;
fixture.track_ephemeral(evidence)?;
```

`FakeExecutable::wait` reaps the parent after its descendant writes the
observable evidence. The test must not replace the barrier with a guessed
duration or leave the child for fixture teardown.

## Crash/restart

- Fixture owner: `recovery`
- Fault point: `journal.after-flush`
- Barrier: `durable-boundary`
- Seed/replay ID: `crash-restart-pattern/6303`
- Capability check: `fixture.require(Capability::UnixPermissions)`; skip only on `Unsupported`
- Evidence bundle path: `<fixture roots>/artifacts/crash-restart-pattern.evidence`

```rust
let mut fixture = LifecycleFixture::create(FixtureSpec::new("crash-restart-pattern", 6303))?;
fixture.require(Capability::UnixPermissions)?;
let mut parent = CrashableParent::spawn(
    &mut fixture,
    CrashSpec::at("journal.after-flush")
        .run_id("crash-restart-pattern-run")
        .with_state("fixture_owner", "recovery"),
)?;
parent.wait_for_boundary()?;
let crash = parent.wait()?;
let retained = RetainedState::restart(&fixture, "crash-restart-pattern-run")?;
assert_eq!(retained.boundary, crash.boundary);

let evidence = fixture.roots().artifacts().join("crash-restart-pattern.evidence");
fs::write(&evidence, format!("owner=recovery\nfault={}\nseed=6303\n", crash.boundary))?;
fixture.track_ephemeral(evidence)?;
```

The durable-boundary marker is the only point at which the test observes the
parent. `RetainedState::restart` reads the fixture-owned journal and exposes
the replay evidence without consulting wall-clock time or ambient state.

## Concurrent fleet

- Fixture owner: `concurrent-fleet`
- Fault point: `run.ready`
- Barrier: `run-ready` for every run, then `release_all`
- Seed/replay ID: `concurrent-fleet-pattern/6404`
- Capability check: `fixture.require(Capability::UnixPermissions)`; skip only on `Unsupported`
- Evidence bundle path: `<fixture roots>/artifacts/concurrent-fleet-pattern.evidence`

```rust
let mut fixture = LifecycleFixture::create(FixtureSpec::new("concurrent-fleet-pattern", 6404))?;
fixture.require(Capability::UnixPermissions)?;
let mut runs = ConcurrentRunControl::launch(
    &mut fixture,
    ["fleet-a".into(), "fleet-b".into(), "fleet-c".into()],
)?;
runs.wait_for_ready()?;
runs.release_all()?;
let results = runs.join()?;
assert!(results.iter().all(|result| result.status.code == Some(0)));

let evidence = fixture.roots().artifacts().join("concurrent-fleet-pattern.evidence");
fs::write(&evidence, "owner=concurrent-fleet\nfault=run.ready\nseed=6404\n")?;
fixture.track_ephemeral(evidence)?;
```

The control starts every run before waiting, releases every ready run in one
explicit operation, and joins all process trees. The stable run IDs and the
fixture event log make a failure replayable without depending on completion
order or a machine's wall clock.

## Anti-flake review checklist

Before adding a lifecycle test, review these points:

1. The fixture case ID and seed are explicit, and the evidence path is inside
   that fixture's artifact root.
2. Every child receives the fixture environment and a fixture-owned working
   directory. No ambient credential or real HOME/config path is used.
3. Faults and barriers have names. The test waits for named state and releases
   it explicitly.
4. Every process tree is waited on and every thread is joined before cleanup.
5. Platform-dependent operations have a capability check with an explicit
   unsupported skip.
6. The test has no wall-clock sleep, polling loop, or retry-based
   synchronization. A source review must reject any such call.
7. The evidence bundle records the owner, fault point, barrier, seed/replay ID,
   and the observed outcome.

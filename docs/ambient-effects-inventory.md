# Ambient-effects inventory

Scope: product code under `src/` (excluding `*_tests.rs`). The inventory records every
remaining ambient call that is not already behind an explicit adapter, with its consumer,
effect, test pain, and whether extraction changes behavior. It distinguishes a necessary
adapter seam from needless abstraction and proposes no public contract or product scope
change.

Status: **no candidates for extraction.** Every ambient effect below is owned by the module
that exercises it; no cross-cutting ambient access remains. Extracting any of these behind a
port would add an abstraction without test or behavior gain.

## Clocks

| Consumer | Effect | Test pain | Extraction changes behavior? |
|---|---|---|---|
| `run_record.rs` — run record timestamp (`SystemTime::now`) | durable wall-clock timestamp on every run record (Constitution Tension 5) | records carry real timestamps; tests assert presence/order, not exact values | No; the record must reflect real wall-clock time — an injected clock is needless abstraction |
| `acquisition.rs`, `capture.rs`, `agent_runtime.rs`, `check_runner.rs` — deadline arithmetic (`Instant::now`) | timeout enforcement, polling budgets, check-run budget | timeout paths already tested via tiny durations and direct deadline math | No |
| `remote_push.rs`, `repair_reserve.rs`, `admission.rs` — deadline arithmetic (`Instant::now`) | push timeout, reserve/repair budget, admission window | timeout paths already tested via tiny durations and direct deadline math | No |
| `fleet_profile.rs`, `fleet_scenarios.rs` — stage/profile timing (`Instant::now`) | profile sampling | profile tests compare relative order, not absolute times | No |

## Environment

| Consumer | Effect | Test pain | Extraction changes behavior? |
|---|---|---|---|
| `adapters.rs` — `PATH` resolution (`var_os`, `split_paths`) | machine-adapter discovery from PATH | already covered by hostile adapter tests with controlled PATH | No — this IS the adapter seam; it owns the effect |
| `invocation.rs` — `HOME` discovery | run-record home resolution | covered by CLI fixture tests with explicit HOME | No — the CLI entry owns home discovery |
| `final_gate.rs` — `CARGO` variable | locating the cargo binary for gate execution | gates are injected in tests; no live cargo invoked | No — the gate runner owns its tool discovery |

## Filesystem

All `std::fs` use is owned by the module that performs the operation (authority roots,
capture, record journal, transaction, setup). No cross-cutting filesystem access exists.

## Process

| Consumer | Effect | Test pain | Extraction changes behavior? |
|---|---|---|---|
| `acquisition.rs`, `agent_runtime.rs`, `remote_push.rs` — `kill` (`Command::new("kill")`) | process-group termination after timeout/cancel | hostile process tests cover kill with explicit argv (`--` before negative pids) | No — each module owns its process lifecycle |
| `release_build.rs`, `release_platform.rs` — `cargo` | clean checkout build/package per target | release fixture tests run the chain with a fixture cargo wrapper | No — release tooling owns its orchestration |
| `release_verify.rs`, `release_gates.rs` — installed binary, gate argv | fresh-install verification, gate orchestration | explicit-argv gate tests | No — release tooling owns its orchestration |
| `check_runner.rs` — configured check argv | executing repository-configured verification commands | hostile check tests with fixture argv | No — the check runner IS the seam; it owns command execution |

## Network

None. The product has no direct network access; remote push/acquire go through the `git`
binary with sanitized environment.

## Git

All git invocation is already behind explicit owned helpers (`sanitized_command`,
`git_text`, release tooling commands) with `GIT_CONFIG_NOSYSTEM=1`,
`GIT_CONFIG_GLOBAL=/dev/null` (product) or per-command `-c commit.gpgsign=false
-c tag.gpgsign=false` (test fixtures). Fixture git no longer inherits the owner's signing
config; the rest of the owner's git config is fully used.

## Terminal

`invocation.rs` diagnostics (`eprintln!`) are owned by the CLI entry; `terminal_projection`
owns the quiet/success projection. No other module writes to the terminal.

## Global state / concurrency

No `GLOBAL_CONFIG`, no static mutables, no global Rayon pool in product code. Fan-out uses
explicit per-run thread pools with owned Arc captures (fleet runner, repair). Test-only
`thread::spawn` in authority tests drives hostile children.

## Conclusion

Per the acceptance criteria, the inventory distinguishes a necessary adapter seam from
needless abstraction: the only existing seams (PATH adapter resolution, sanitized git
commands, injected final-gate arguments) are necessary; every remaining ambient effect is
module-owned, already tested at its boundary, and would not change behavior if extracted.
No candidates — the container's optional extraction work is vacuous by design.

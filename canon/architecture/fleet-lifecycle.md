---
status: reference
scope: [fleet-execution, verification, repair, migration]
related:
  - configuration-authority.md
  - managed-content.md
---

# Fleet lifecycle

## Independent repository outcomes

Destination repositories are independent work units. Omnirepo processes them
concurrently where possible; completion order has no significance.

Every repository receives a final success or failure outcome. A repository
failure does not stop valid peers, and successful peers proceed through their
full lifecycle. Failures are collected for repair and final reporting rather
than ending the fleet run at the first error.

## Bounded scheduling

Each run freezes the machine limits and uses one scheduler; completion order does not alter authority or accounting.
A repository slot is held from destination admission through its terminal initial-pass or repair outcome, with at most `max_repositories` slots live.
Source snapshots are deduplicated per source ID; acquisition uses child work but no destination slot, so repositories wait without holding a slot.
Destination synchronization, verification, Git, and repair-agent work is serialized per canonical repository and never overlaps for that repository.
Child work is one global permit pool bounded by `max_child_work`, with source, verification, Git, and agent resource kinds.
Source work has one operation per source ID; checks run one at a time in configured order; each Git operation and agent invocation is one at a time per repository.
An agent process tree and its contained descendants use one agent permit; adapter-specific timeouts remain the adapter contract.
The fixed order is cancellation and journal health, source readiness or lease, stage intent, repository slot, child permit, child process, then result.
Stage intent is acknowledged before a repository or child permit is taken; no permit is held while waiting for a slot or journal intent, and no child operation acquires a second permit.
After a child process tree terminates and is reaped, its permit returns before the repository stage is requeued or advanced.
Ready source and stage requests use a bounded FIFO queue with one pending request per source or repository; a requeued repository goes to the tail.
Journal saturation pauses new admission and effects without dropping outcomes; writer failure stops new effects and emits typed scheduler or cancellation results.
Cancellation closes admission, terminates and reaps active trees, releases permits and slots only after termination, and terminalizes queued repositories.
Scheduler events record tickets, resource kinds, active and queued counts, high-water marks, backpressure, cancellation, release, and terminal outcomes.
The scheduler uses explicit permits and workers; ambient Rayon pools, CPU autodetection, and environment-based concurrency are not authoritative.

## Per-repository lifecycle

For each destination repository, the initial fleet pass:

1. applies the authoritative managed-file and managed-section changes;
2. runs the verification commands declared by that repository;
3. commits and pushes the current synchronization changes only after the
   configured checks pass; and
4. collects synchronization and verification failures for a separate repair
   operation after the pass.

A repair attempt reruns synchronization and configured verification as a new
repository pass before any commit or push. A repository whose sync,
verification, repair, commit, or push still fails after its permitted repair
attempts is a failure. Omnirepo must not commit or push while configured checks
fail.

## Verification commands

Repository checks are explicit argument arrays and never shell strings. They
run from the destination repository root in configured order from one frozen
policy snapshot. Exact duplicate command entries are invalid. An absent or
empty command list means that no verification command is required; Omnirepo
does not invent defaults.

Every configured check is attempted even after an earlier check fails. Standard
input is closed; standard output and standard error are captured separately.
Checks inherit a sanitized process environment and ordinary network access, plus
only repository-declared additional environment variables. The default timeout
is ten minutes per check and repository policy may set a finite override. A
timeout or cancellation terminates and reaps the complete process tree.
Spawn errors, nonzero exits, signals, timeouts, and cancellation are distinct
typed failures retained for repository accounting.

Verification commands may read their destination repository and use ordinary
network access, but receive no ambient secrets. They may leave only ignored or
untracked ephemeral build artifacts. A check that changes managed bytes,
repository policy, tracked or staged content, Git authority, machine or source
authority, run records, another repository, or anything outside its destination
fails verification. Those changes are excluded from the authorized delta and
restored; inability to confine, terminate, or restore the command is a failure.
Late writers are treated as part of the command process tree.

## Repository transaction residue

All operations targeting one destination file form one atomic group. Groups are
preflighted and attempted in deterministic order; failure of one group does not
prevent independent groups in that repository from being attempted. Successful
authoritative group writes remain visible when another group or a later
lifecycle stage fails. There is no repository-wide rollback.

Verification failure retains the authorized synchronization writes but produces
no Omnirepo commit. A failed repair attempt restores all agent-created changes
to its attempt baseline while retaining authoritative synchronization writes.
A local commit failure leaves the authorized worktree changes and preserves the
pre-existing index; a push failure may leave the exact local operation commit.
Forbidden, concurrent, or late verifier or agent deltas are never retained as
authorized work and never enter Git. Crash and cancellation residue follows the
durable recovery journal.

## Git delivery

Omnirepo uses the invoking user's configured Git author identity,
authentication, and signing policy. Repository hooks, URL rewrites, filters,
and other ambient mechanisms that could widen effects are disabled. Missing
identity or signing capability is a repository failure rather than a reason to
change identity or bypass the selected signing policy.

Unrelated worktree and index state may coexist only when it does not overlap the
current synchronization targets. Omnirepo uses an isolated index and stages
only the authorized current-run delta, preserving unrelated state exactly. It
creates at most one commit per repository and run, with the message
`chore(omnirepo): sync managed content`. An unchanged repository creates no
commit.

The repository must be on its configured branch with a configured upstream and
must not already be ahead, detached, diverged, or moved from the frozen base.
Omnirepo pushes only the recorded operation commit OID to the frozen remote and
ref, never forces and never publishes incidental refs or pre-existing commits.
Each push attempt has a two-minute timeout and at most two retries. A timeout or
disconnect is reconciled against the remote OID before retry, so delivery is
idempotent and cannot create a duplicate commit.

## Agent-assisted repair

Repair is a separate, causally bounded operation; it does not expand
synchronization ownership. The first release has built-in adapters for Codex,
Claude Code, and Pi. Omnirepo discovers compatible executables on `PATH` and
selects them in machine-configured priority order. There is no CLI priority
override and no arbitrary-command adapter. Missing or incompatible adapters are
recorded and skipped; an empty or exhausted list leaves the repository failed.

The adapter freezes the canonical executable identity and capability/version
before invocation and fails on replacement races. A structured JSON control
protocol separates trusted harness fields from untrusted repository content and
agent output. Each invocation is non-interactive, has closed user input, a
fifteen-minute timeout, and complete process-tree termination. The adapter's
own authentication may be used only to reach its agent service; other user,
Git, cloud, SSH, helper, proxy, and repository credentials are unavailable.

An agent receives one destination worktree, the authorized current-run delta,
bounded verification evidence, and frozen identities. It may use uncredentialed
outbound network access and contained subprocesses. Other fleet repositories,
source repositories, machine configuration, run-record storage, and secret
stores are inaccessible. Git metadata is read-only to the agent; commit and
push remain Omnirepo lifecycle operations. If this confinement cannot be
enforced, no agent is invoked.

A verification failure is repair-eligible only when causation is established by
either the same frozen checks passing at an identical pre-sync baseline or
prior run, or deterministic failure evidence that directly identifies a
managed path changed by the current run. Exact managed-content application
failures may also qualify when their current-run causation is direct. Uncertain
causation never reaches an agent.

A repair agent may change destination-local content, including content outside
managed files or sections, only when necessary to correct that regression. It
may not alter machine-level fleet membership, authoritative source content, or
source priority as described in
[configuration-authority.md](configuration-authority.md).

When causation is unrelated or uncertain, a proposed action is outside the
project boundary, or permitted attempts are exhausted, repair aborts and the
repository remains failed. Command failures and agent-run evidence are retained
for the durable run summary.

The machine default and maximum are three attempts per failed repository; a
repository may lower that number. An attempt is durably reserved before agent
contact and is consumed by success, failure, timeout, crash, or interruption.
Fallback follows configured adapter order and restart preserves consumed
reservations. Machine/source/configuration/catalog/plan, journal, Git commit or
push, cancellation, crash, unrelated, and uncertain failures are not
agent-repairable. After every candidate repair, Omnirepo reapplies authoritative
synchronization and reruns the frozen checks before Git delivery. Agent output
never establishes success or causation by itself.

## Records and reporting

Successful synchronization and permitted recovery changes are recorded in
ordinary Git commits and pushed. Every fleet run also writes a durable run
summary under
`<HOME>/.omnirepo/runs/<UTC-timestamp>-<128-bit-random>.log`. The file is an
exclusively created, mode-`0600`, versioned JSON Lines journal followed by one
terminal summary. It contains complete repository accounting, including named
failures and their collected verification and repair evidence. Routine success
output stays concise.

A syntactically valid `sync` invocation becomes a fleet run before machine
configuration is loaded or any source or destination effect begins. The
journal must be created before those effects. Help, version, parse errors,
`setup`, and `validate` are not fleet runs. If the journal cannot be created,
the run exits before source or repository effects. A corrupt, truncated, or
unfinalized journal is never interpreted as successful.

Captured evidence is sanitized before persistence or display. Each check or
agent attempt retains at most one mebibyte across its bounded output channels,
with explicit truncation markers. Known credential values, URL userinfo,
authentication material, and terminal/control sequences are redacted or
escaped, including across chunk boundaries. Routine terminal output does not
dump raw evidence. Run records are retained for thirty days and then safely
removed; retention never changes Git or repository content.

## Interruption and overlapping runs

Repository admission uses canonical per-repository leases. A second run that
targets a busy repository records that repository as busy and continues with
independent peers rather than waiting. A stale lease is recovered only after
the prior owner is proven dead and its durable journal is reconciled.

After interruption, Omnirepo replays durable stage intent and observes Git and
remote state before deciding whether an effect completed. It resumes a
repository only when the frozen source, configuration, plan, check, base-HEAD,
and remote identities still match. Otherwise it terminalizes the interrupted
repository as failed and a later run starts from a fresh plan. Recovery never
duplicates a write, commit, push, or repair attempt.

Cancellation stops new repository admission, closes child input, requests
termination, and force-terminates remaining process trees after five seconds;
a repeated cancellation may force immediately. Every selected repository,
including queued and interrupted members, receives a terminal cancellation or
failure outcome when the journal remains writable. Already completed Git
effects are reconciled and retained rather than rolled back.

## Public command outcomes

The public command surface is `sync`, `setup`, and `validate`. Humans and
agents invoke the same commands. Human output is quiet on routine success; a
versioned `--output json` mode provides the same outcomes without progress or
diagnostic contamination.

Process exit codes are stable: `0` for success, `2` for invocation or shared
configuration failure, `3` when some repositories fail and some succeed, `4`
when every selected repository fails, `5` when the durable run record cannot be
created or finalized, and `130` for user cancellation. Help, version, and
argument parsing do not require configuration and create no run record or
repository effect. A record failure never prints a false record pointer.

## Breaking migrations

A breaking Omnirepo release must identify the break and provide actionable,
release-bound migration guidance. The first constitutional release does not
provide an automated migration artifact, migration agent, or `migrate` command.
Installation, update, configuration loading, setup, validation, and
synchronization never migrate destination repositories or configuration
implicitly.

Automated migration remains a possible later extension only after a new owner
decision explicitly delegates it within the constitutional migration boundary.

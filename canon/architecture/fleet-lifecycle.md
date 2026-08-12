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

## Agent-assisted repair

Repair is a separate, causally bounded operation; it does not expand
synchronization ownership. Omnirepo discovers available AI agent CLIs and
selects them in the user's configured priority order, subject to configured
repair limits and agent-harness settings. Each repair attempt is confined to a
regression caused by the current synchronization.

A repair agent may change destination-local content, including content outside
managed files or sections, only when necessary to correct that regression. It
may not alter machine-level fleet membership, authoritative source content, or
source priority as described in
[configuration-authority.md](configuration-authority.md).

When causation is unrelated or uncertain, a proposed action is outside the
project boundary, or permitted attempts are exhausted, repair aborts and the
repository remains failed. Command failures and agent-run evidence are retained
for the durable run summary.

## Records and reporting

Successful synchronization and permitted recovery changes are recorded in
ordinary Git commits and pushed. Every fleet run also writes a durable run
summary under
`<HOME>/.omnirepo/runs/<timestamp>.log` with complete repository accounting,
including named failures and their collected verification and repair evidence.
Routine success output stays concise.

## Breaking migrations

A breaking Omnirepo release must identify the break and provide actionable,
preferably automated migration instructions. When explicitly delegated, a
separately invoked migration operation may select from the same discovered,
configured agents used for repair, solely to apply the declared Omnirepo
migration to a destination repository. It may not alter machine-level fleet
membership, authoritative source content, or source priority. Migration does
not authorize deployment, dependency management, release work, secret
management, or unrelated repository maintenance.

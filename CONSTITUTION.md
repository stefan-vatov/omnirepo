# Omnirepo Constitution

## Preamble

Software creation is no longer constrained to a handful of projects. With AI,
one person or team can build and operate tens or hundreds—but the burden of
maintaining them multiplies just as quickly. Every project accumulates
configuration, hooks, agent instructions, and other shared foundations that
must evolve as tools and practices improve.

Omnirepo exists so an improvement can be made once and carried reliably across
an entire project portfolio. It removes the repetitive work of finding every
repository, applying the same change by hand, and wondering which projects have
drifted. Its purpose is to return that attention to meaningful product work.

Consistency does not require every project to be identical. Omnirepo may govern
an entire file or only a clearly managed section, leaving local content free to
differ. Within those declared boundaries, the shared definition is
authoritative: running Omnirepo restores managed content even when local copies
have changed.

## Founding Principles

1. **Managed means authoritative.** Within a declared managed file or section,
   Omnirepo's source replaces local drift without confirmation; unattended
   convergence is the product. *Rejects: preserving unauthorized local edits,
   interactive approval loops, and treating managed content as a suggestion.*

2. **Convention governs until intent is declared.** A repository without
   configuration receives useful inferred synchronization. Once repository
   configuration exists, it wholly governs that repository's managed file and
   section scope and its repository commands. Machine configuration alone
   governs fleet membership, authoritative sources, and source priority; all
   configuration remains subordinate to this constitution's boundaries.
   *Rejects: mandatory repetitive setup, inference that overrides repository
   policy, and destination configuration that redefines fleet-wide authority.*

3. **Shared synchronization authority must have visible edges.** The sync
   engine may govern an entire file or a clearly delimited section; outside
   those boundaries, source parity has no authority. Destination-local repair
   or migration is a separate operation governed by the repair boundary, not an
   expansion of synchronization ownership. *Rejects: ambiguous ownership,
   invisible partial management, and using repair as implicit authority over a
   repository.*

4. **Exact text outranks semantic cleverness.** Omnirepo reproduces managed
   content faithfully and understands file types only enough to mark its
   boundaries; integration problems belong to an explicit repair step.
   *Rejects: tool-specific merge engines, speculative rewriting, and silently
   "improving" authoritative source content.*

5. **The fleet advances despite stragglers.** Repositories are independent
   units: successful work proceeds and is recorded while failures are
   collected, reported, and repaired afterward. *Rejects: fleet-wide
   all-or-nothing transactions, stopping at the first failure, and sacrificing
   broad progress for completion order.*

6. **Precedence is declared, never guessed.** When sources overlap, their
   configured order is the sole tiebreaker; authority must remain simple enough
   to predict without inspecting implementation details. *Rejects:
   content-based heuristics, hidden priorities, and intelligent conflict
   arbitration.*

7. **Humans and agents operate the same system.** Omnirepo gives both comparable
   capabilities, uses configured agent tools for recovery, and relies on
   ordinary Git history to make every resulting change inspectable and
   reversible. *Rejects: opaque agent-only behavior, separate automation
   realities, and bespoke provenance beyond the repository's normal record.*

8. **Evolution may break interfaces, but must not abandon the fleet.** Omnirepo
   may change its CLI, configuration, or model when unattended file and section
   maintenance is better served, provided the break is explicit and accompanied
   by actionable—preferably automated—migration guidance. Semantic migration
   may be explicitly delegated, but a breaking change may never expand Omnirepo
   beyond its constitutional purpose. *Rejects: permanent compatibility at the
   expense of the mission, unmarked breaking releases, manual fleet-wide
   migrations, and migration as an excuse for scope expansion.*

## Growth Directives

1. **Toward effortless adoption.** Reduce the distance between installation and
   a first successful unattended synchronization through useful inference,
   clear setup, and minimal configuration.

2. **Toward quiet fleet-scale operation.** Make synchronizing tens or hundreds
   of repositories fast, smooth, and silent when everything succeeds, while
   preserving a concise account of anything that does not.

3. **Toward resilient convergence.** Improve automatic recovery from
   synchronization-induced failures, including agent-assisted repair, so large
   fleets return to parity with less human intervention.

4. **Toward broader textual compatibility.** Support more common file types
   where delimiter knowledge is needed for partial management, while continuing
   to treat managed content as text rather than growing format-specific
   semantics.

5. **Toward less product, better executed.** Grow by making file and section
   synchronization cleaner, faster, and easier—not by expanding into general
   repository orchestration.

## Boundaries

1. **Never become a general repository orchestrator.** Omnirepo synchronizes
   authoritative files and sections into destination repositories. Committing,
   pushing, checking, and repairing sync-induced regressions complete that
   synchronization; deployments, dependency management, releases, secrets, and
   unrelated maintenance remain outside its purpose.

2. **Never reverse the flow of authority.** Synchronization moves strictly from
   ordered source repositories to destinations. Omnirepo does not learn from,
   merge back, or promote destination changes into a source.

3. **Never let inference overrule declared intent.** Inference exists only where
   repository policy is absent. Once a repository has configuration, Omnirepo
   follows it rather than inventing a broader or "smarter" scope.

4. **Never turn the sync engine into a semantic configuration editor.** The
   sync engine may understand comment delimiters well enough to maintain partial
   sections, but otherwise treats managed content as exact text. Semantic
   changes belong only to an explicitly delegated repair or migration operation
   governed by Boundary 5; they are never inferred as part of synchronization.

5. **Never claim ownership of unrelated repository health.** A separately
   invoked repair or migration agent may change destination-local content—including
   content outside managed files or sections—only when needed to correct a
   regression caused by the current synchronization or to apply a declared
   Omnirepo migration. It may not alter machine-level fleet membership,
   authoritative source content, or source priority. When causation is unrelated
   or uncertain, the action is unlawful, or permitted attempts are exhausted, it
   aborts and reports the repository as failed.

## Tension Pairs

1. **Unattended synchronization convergence over preserving local drift—but
   never at the cost of changing repositories outside the machine-declared fleet
   or allowing the sync engine itself to cross managed partial delimiters.**
   Destination-local repair or migration beyond those delimiters is a separate,
   causally bounded operation governed by Boundary 5.

2. **Repository-specific applicability over universal treatment—but never at
   the cost of letting a destination redefine the authoritative sources or their
   priority.** The local configuration controls all, allow, and exclude scope and
   repository commands; the machine configuration controls the fleet and source
   order.

3. **Parallel speed over deterministic completion order—but never at the cost
   of configured verification or complete accounting of every repository's
   outcome.**

4. **Independent fleet progress over all-or-nothing consistency—but never at
   the cost of committing or pushing a repository whose configured checks still
   fail after the permitted repair attempts.**

5. **Quiet success over operational chatter—but never at the cost of ordinary
   Git history and a durable, timestamped record of every run.**

6. **Exact textual parity over semantic cleverness—but never at the cost of
   respecting managed-section boundaries or reporting integration failures for
   bounded repair.**

## Amendments

This constitution was ratified on August 12, 2026.

Only the project owner may initiate, review, approve, or reject an amendment.
Amendment is never automatic, scheduled, contributor-initiated, or
agent-suggested. Every other amendment path is rejected.

An amendment is ratified by the project owner's deliberate approval in a
human-invoked constitution session. It takes effect immediately upon approval.

Git history is the sole amendment record. The constitution contains no separate
amendment log; version control preserves the changed section, previous and new
text, date, and rationale.

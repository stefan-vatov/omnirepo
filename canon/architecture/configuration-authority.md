---
status: reference
scope: [configuration, authority, precedence]
---

# Configuration authority

## Machine configuration

Machine configuration lives at `<HOME>/.omnirepo/config.yaml`. YAML is the
only supported configuration serialization. The machine configuration is the sole
authority for:

- the destination repositories in the fleet;
- the ordered source repositories that provide managed content; and
- source priority when sources provide overlapping managed content; and
- the bounded repository-admission and child-work concurrency limits; and
- the ordered repair-agent adapters and the global repair-attempt ceiling.

Source repositories contain authoritative managed content and declare the files
and sections they provide in `<source-root>/.omnirepo/source.yaml`. Each
declaration has an explicit stable lowercase-slug ID, a whole-file or section
mode, contained source and destination paths, a section ID (required for
section mode, forbidden for whole-file mode), and optional
destination-repository applicability tags. A declaration inherits the exact
revision pinned for its source snapshot and does not repeat that revision in
its own content. Managed bytes remain in
ordinary source files rather than being embedded in YAML; a section's body is
the exact bytes of its declared source file. When sources
overlap, their configured order is the sole tiebreaker. Completion order,
content inspection, and inferred importance must not change precedence.

Machine configuration does not define destination-repository commands or the
managed scope inside a destination repository.

### Machine concurrency

The machine configuration may contain one `concurrency` mapping. Its fields
are machine-owned and apply to the complete fleet run:

```yaml
concurrency:
  max_repositories: 4
  max_child_work: 8
```

`max_repositories` is a `u16` unsigned integer in `1..=32`, with a default of
`4`. It bounds concurrently admitted destination-repository lifecycles.
`max_child_work` is a `u16` unsigned integer in `1..=64`, with a default of `8`.
It bounds the global machine-run permits for nested child work. These defaults
are fixed; Omnirepo does not infer them from CPU count, memory, environment, or
an ambient worker pool. The mapping or either field may be omitted when the
machine authority file is otherwise valid. Explicit zero, negative, fractional,
non-integer, null, out-of-range, unknown, or duplicate values are errors. Zero
never means unbounded, automatic, or disabled scheduling.

`sync` may receive transient `--max-repositories N` and
`--max-child-work N` overrides. Each supplied value must be a positive integer
no greater than the corresponding validated machine value; it can only lower
the cap for that run. An omitted override uses the machine value. An invalid or
larger override is an invocation error, and neither it nor any destination
configuration can raise or replace machine authority. The overrides are not
persisted and are not accepted as repository or source configuration fields.
The effective caps are frozen for the run and completion order cannot change
source precedence, fleet membership, or repository outcome accounting.

Agent settings are operational recovery policy. They cannot change fleet
membership, authoritative source content, source order, repository managed
scope, or repository verification commands. Repository policy may only lower
its own repair-attempt limit.

## Repository configuration

A destination repository may declare `.omnirepo.yaml` at its root. That
configuration owns only that repository's applicability and commands: it may
select all managed content, allow selected content, exclude selected content,
or mix those controls.

For explicit selection, the selected set is all applicable content when
`all` is enabled, union exact `allow` ID matches, minus exact `exclude` ID
matches. Exclusion wins. A present empty, omitted-selector, or commands-only
configuration selects no managed content and does not invoke inference.
Unknown or duplicate selectors are errors. Selection does not inspect whether
destination files exist or what they contain.

When repository configuration exists, it is intentional and wholly governs
that repository's managed scope and commands. Inference must not broaden or
override it. Repository configuration cannot alter fleet membership,
authoritative sources, or source priority.

When repository configuration is absent, Omnirepo infers every applicable
source declaration it can match for that repository and synchronizes it. This
convention is a fallback, not an authority above explicit repository intent.
Applicability uses only stable repository tags declared by machine
configuration. It never probes destination content.

## Configuration discovery

The three canonical paths are `<HOME>/.omnirepo/config.yaml`, destination-root
`.omnirepo.yaml`, and source-root `.omnirepo/source.yaml`. Discovery checks only
those exact files. Schemas reject unknown fields and duplicate YAML keys.
Malformed, unreadable, non-regular, aliased, competing, or legacy authority
files are errors rather than absence or fallback. The CLI cannot substitute a
different machine authority file.

Configured paths use UTF-8 strings with `/` separators. Authority roots may be
absolute; paths declared within a source or destination root are relative and
must remain contained by that root.

## Schema evolution

Every machine, source, and destination-repository configuration requires the
integer field `version: 1`. A missing, non-integer, older, or future version is
unsupported and fails without changing configuration or repository content.
Mixed supported and unsupported authority files do not receive partial or
implicit migration. The error identifies the unsupported configuration and
provides actionable migration guidance.

Configuration loading, validation, setup, and synchronization never migrate a
configuration implicitly.

## Setup

Setup first computes and displays an effect plan. Interactive use requires an
explicit terminal confirmation before applying that plan. Non-interactive use
is prompt-free and requires `--yes` together with every required input.

Setup may create or update only explicitly selected canonical machine, source,
or destination-repository configuration files. It never discovers an ambient
fleet, creates repositories, or replaces invalid or conflicting authority.
Applying the same valid setup intent repeatedly is a no-op. A failed apply must
not leave ambiguous configuration authority.

## Direction

Authority flows one way: from ordered source repositories to destination
repositories. Destination content is never learned from, merged back into, or
promoted to an authoritative source.

## Source materialization

A machine source entry is either a local Git repository or a remote Git
repository reached through standard HTTPS or SSH authentication. A local source
must be a clean worktree on `main`; Omnirepo pins its current `HEAD` and never
pulls or rewrites it.

A remote source is maintained beneath the machine-configured cache root.
Before each run, Omnirepo fetches `main`, creates an immutable snapshot of the
fetched commit, and never uses a stale cache as offline authority. One fetch
attempt is followed by at most two retries with bounded backoff; each attempt
has a two-minute timeout. A missing, corrupt, wrong-remote, or unrecoverable
cache may be discarded and cleanly cloned again. Cache recovery never changes
the remote source or a destination repository.

Source acquisition may use the user's ordinary HTTPS credential helpers or SSH
agent. Source-controlled hooks, filters, submodules, LFS behavior, executable
protocol helpers, and URL rewrites are disabled. An unavailable higher-priority
source is retained as an explicit failure and never silently promotes a lower
source into its authority.

# Omnirepo reference

## CLI reference

The public command surface is exactly `sync`, `setup`, and `doctor`.

- `omnirepo sync` — a fleet run: creates the durable record at the
  invocation boundary, builds the source catalog and the per-repository
  plans, applies the managed changes, runs the declared verification,
  delivers one scoped commit per repository after the checks pass, runs
  bounded repair for eligible failures, finalizes the record, and
  exits with the stable code.
- `omnirepo setup [--apply]` — authors the canonical machine
  configuration; displays the effect plan first; apply requires an
  explicit confirmation (interactive) or `--yes` (non-interactive);
  repeated apply is a no-op; an invalid or conflicting authority is
  never replaced.
- `omnirepo doctor` — the machine diagnostic without effects: runs the
  same effect-free planning prefix as `sync` (machine configuration,
  source catalog, pinned declarations, repository policies, bindings,
  per-repository plans) and reports source availability, every managed
  item and its section, every shadowed loser with its winner, and
  declarations that would fail at sync time. Doctor reads only each
  destination's `.omnirepo.yaml` repository policy, never managed
  content; it writes nothing and is never a fleet run. Exit `0`
  healthy, `2` problems.
- `--output human|json` — the global output selector (quiet human by
  default; versioned machine-readable JSON projection).

Help, version, and argument parsing require no configuration and create
no run record or repository effect. `sync` and `doctor` are
prompt-free; setup follows its explicit confirmation contract.

### Exit codes

| Code | Meaning |
|------|---------|
| 0 | Success, including unchanged and empty fleets |
| 2 | Invocation or shared-configuration/preflight failure |
| 3 | Some repositories failed, some succeeded |
| 4 | Every selected repository failed |
| 5 | Durable-record create or finalize failure |
| 130 | User cancellation |

A record failure never prints a false record pointer.

## Configuration reference

The three canonical configuration paths are exact; nothing else is
scanned:

- `<HOME>/.omnirepo/config.yaml` — the machine configuration: the
  destination fleet, the ordered sources, source priority, the
  concurrency limits, and the repair policy.
- `<source-root>/.omnirepo/source.yaml` — the source declarations:
  stable lowercase-slug IDs, whole-file (`mode=sync`) or section
  (`mode=section`) mode, contained source and destination paths, the
  section ID (`section=<id>`, required for section mode, forbidden
  otherwise), optional destination tags. For a section, the whole
  source file is the section body.
- `<destination-root>/.omnirepo.yaml` — the repository policy:
  all/allow/exclude selection (exclusion wins) and the declared
  verification commands (explicit argument arrays, never shell
  strings).

Every configuration requires `version: 1`. Malformed, unreadable,
non-regular, aliased, competing, or legacy authority files are errors,
never absence. Absent repository policy infers every applicable
declaration; present policy wholly governs that repository.

### Concurrency

```yaml
concurrency:
  max_repositories: 4   # u16 in 1..=32, default 4
  max_child_work: 8     # u16 in 1..=64, default 8
```

The caps freeze for the run; the transient `--max-repositories` and
`--max-child-work` sync overrides may only lower them and are never
persisted.

## Delimiter reference

Managed partial sections use exact full-line named delimiters in the
destination format's comment syntax; the section ID names the block:

| Format | Open | Close |
|--------|------|-------|
| yaml / toml / shell | `# omnirepo:start <section-id>` | `# omnirepo:end <section-id>` |
| python / ruby | `# omnirepo:start <section-id>` | `# omnirepo:end <section-id>` |
| json / javascript / typescript | `// omnirepo:start <section-id>` | `// omnirepo:end <section-id>` |
| rust | `// omnirepo:start <section-id>` | `// omnirepo:end <section-id>` |
| markdown / html | `<!-- omnirepo:start <section-id> -->` | `<!-- omnirepo:end <section-id> -->` |
| ini | `; omnirepo:start <section-id>` | `; omnirepo:end <section-id>` |
| sql | `-- omnirepo:start <section-id>` | `-- omnirepo:end <section-id>` |

The destination format resolves from the destination path's extension
(last dot-separated component, lowercase). Section IDs use ASCII
lowercase letters, digits, dots, underscores, and hyphens, exact and
case-sensitive.

Multiple non-overlapping named sections may share one destination file;
each is replaced independently and an absent section is appended after
the existing content with one separating blank line. Managed content is
exact text: byte-identical reproduction, no normalization, no semantic
merge. Outside the delimiters — including sections owned by other
declarations — content is preserved verbatim. Unnamed,
whitespace-altered, nested, interleaved, unpaired, duplicate-ID, or
payload-like marker lines are typed failures, never inferences, and
leave the file unchanged.

## Operation reference

1. **Invocation** — a syntactically valid `sync` becomes a fleet run
   before any configuration or effect; the durable record is created
   first.
2. **Catalog** — the sources record Complete (typed root open +
   pinned revision) or Unavailable; an unavailable higher-priority
   source never promotes a lower source.
3. **Plans** — per destination: declarations bound by applicability,
   policy loaded with lawful absence, the immutable plan built in
   source precedence then declared order.
4. **Pass** — per admitted repository: apply the managed changes, run
   the declared checks in configured order, commit and push only after
   the checks pass. At most one commit per repository per run with the
   message `chore(omnirepo): sync managed content`; an unchanged
   repository creates no commit; a failure never stops its peers.
5. **Repair** — eligible failures with proven causation reserve exactly
   one durable attempt and run the confined agent (machine default of
   three attempts); uncertain causation never reaches an agent; after
   repair the authoritative sync is reapplied and the frozen checks
   rerun before delivery.
6. **Finalize** — the terminal record is written, the quiet human or
   JSON projection is rendered, and the exact exit code is returned.

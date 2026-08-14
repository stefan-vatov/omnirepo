# Omnirepo reference

## CLI reference

The public command surface is exactly `sync`, `setup`, and `validate`.

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
- `omnirepo validate` — validates machine configuration and repository
  policy without effects.
- `--output human|json` — the global output selector (quiet human by
  default; versioned machine-readable JSON projection).

Help, version, and argument parsing require no configuration and create
no run record or repository effect. `sync` and `validate` are
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
  stable lowercase-slug IDs, whole-file or section mode, contained
  source and destination paths, optional section ID, optional
  destination tags.
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

Managed partial sections use comment delimiters per file format:

| Format | Open | Close |
|--------|------|-------|
| yaml / toml / shell | `# omnirepo-start` | `# omnirepo-end` |
| json / javascript / typescript | `// omnirepo-start` | `// omnirepo-end` |
| markdown / html | `<!-- omnirepo-start -->` | `<!-- omnirepo-end -->` |
| python | `# omnirepo-start` | `# omnirepo-end` |
| rust | `// omnirepo-start` | `// omnirepo-end` |

Managed content is exact text: byte-identical reproduction, no
normalization, no semantic merge. Outside the delimiters, content is
preserved verbatim. An ambiguous or missing section is a typed failure,
never an inference.

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

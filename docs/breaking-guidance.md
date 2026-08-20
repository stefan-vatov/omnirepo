# Breaking migration guidance

This release is the first constitutional release. It breaks the legacy
0.8.x surface. Every break below is actionable: what changed, how to
migrate, and what happens if you do not.

## Migration policy

Automated migration is declined for the first constitutional release:
there is no automated migration artifact, no migration agent, and no
`migrate` command. Installation, update, configuration loading, setup,
validation, and synchronization never migrate configuration or
destination repositories implicitly. Actionable manual migration
guidance is mandatory and provided here. Automated migration may be
reconsidered only through a later explicit owner decision.

## Break 1: the general multi-repository run surface

**What changed.** The legacy `run` surface accepted arbitrary
repositories. The constitutional surface is `sync` over the
machine-declared fleet only.

**How to migrate.** Declare your fleet in
`<HOME>/.omnirepo/config.yaml`:

```yaml
version: 1
repositories:
  - id: destination-a
    path: /srv/repositories/a
```

Then run `omnirepo sync`.

**If you do not migrate.** `run` fails with an argument error; no
general multi-repository orchestration exists.

## Break 2: the tag-based clone workflow

**What changed.** The legacy tag-clone workflow is removed.

**How to migrate.** Declare the source and its pinned revision in the
machine configuration; the source declares its managed content in
`<source-root>/.omnirepo/source.yaml`; `sync` pins and uses the
canonical revision.

**If you do not migrate.** No tag-clone command exists; source content
arrives only through the declared source declarations.

## Break 3: ad hoc synchronization authority

**What changed.** Arbitrary source/destination pairs are no longer
accepted.

**How to migrate.** Declare every source and destination in the
canonical machine configuration; authority flows strictly from ordered
sources to destinations.

**If you do not migrate.** Ad hoc pairs fail typed; no destination is
changed outside the declared fleet.

## Break 4: the legacy configuration loader

**What changed.** The 0.8.x `Config`/`RepoConfig` template authority and
its ambient config loaders are removed.

**How to migrate.** Author the canonical configuration with `setup` or
by hand at the three canonical paths; every file requires
`version: 1`.

**If you do not migrate.** Legacy config files are never discovered;
invalid or conflicting authority is an error, never a fallback.

## Break 5: general orchestrator, deployment, and dependency surfaces

**What changed.** Surfaces outside the constitutional purpose
(deployments, dependency management, releases, secrets, unrelated
maintenance) never existed and never will.

**How to migrate.** Use your existing deployment and dependency tooling;
Omnirepo synchronizes authoritative files and sections only.

**If you do not migrate.** Nothing changes: those surfaces remain
outside the product.

## Break 6: legacy logging, progress, table, and verbose flags

**What changed.** The legacy output flags are removed.

**How to migrate.** Use the quiet human output or `--output json` for a
versioned machine-readable projection.

**If you do not migrate.** The flags fail with an argument error; the
record still carries the complete accounting.

## Break 7: named partial-section markers {#named-markers}

**What changed.** Partial sections use exact full-line named markers
(`<comment-token> omnirepo:start <section-id>` /
`omnirepo:end <section-id>`). The old unnamed `omnirepo-start` /
`omnirepo-end` markers are no longer a marker format, and source files
no longer carry markers at all: the whole source file is the section
body, and the declaration names the section with a required
`section=<id>` field.

**How to migrate.** In each source's `.omnirepo/source.yaml`, add
`section=<id>` to every `mode=section` declaration and strip the old
marker lines from the source partial files. In destination files, remove
the legacy `omnirepo-start`/`omnirepo-end` blocks (or rename them by
hand to the named form); the next `sync` writes the named sections
itself.

**If you do not migrate.** A destination file that still carries legacy
marker lines fails its group typed ("legacy unnamed marker"), never
silently: omnirepo refuses to append a named section beside a stale
legacy block. A `mode=section` declaration without `section=` fails
binding typed.

## Break 8: `validate` replaced by `doctor` {#doctor}

**What changed.** The `validate` stub is removed. `doctor` is the one
machine diagnostic: it runs the effect-free planning prefix of `sync`
and reports source availability, managed items, shadowed losers, and
declarations that would fail at sync time.

**How to migrate.** Run `omnirepo doctor`. Exit `0` is healthy; exit
`2` means problems were found.

**If you do not migrate.** `omnirepo validate` fails with an argument
error.

## Optional migration policy

Optional P2 capabilities (automated migration, resource limits,
model-based testing, ambient-adapter refactoring) remain outside the
mandatory release unless the owner explicitly promotes them. Promotion
is a recorded owner decision, never an agent inference.

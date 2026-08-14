# Quickstart

This walkthrough takes you from an empty machine to a first unattended
synchronization.

## 1. Install

```sh
cargo build --release --locked
export PATH="$PWD/target/release:$PATH"
```

## 2. Author the machine configuration

The canonical machine configuration lives at
`<HOME>/.omnirepo/config.yaml`. `setup` authors it for you:

```sh
omnirepo setup --apply
```

Setup first displays the effect plan and requires an explicit
confirmation (interactive) or `--yes` (non-interactive). Applying the
same intent repeatedly is a no-op; an invalid or conflicting authority
is never replaced.

A minimal configuration:

```yaml
version: 1
repositories:
  - id: destination-a
    path: /srv/repositories/a
sources:
  - id: upstream
    location: https://example.com/repo.git
concurrency:
  max_repositories: 4
  max_child_work: 8
```

Sources declare their managed content in
`<source-root>/.omnirepo/source.yaml`. Each declaration has a stable
ID, a whole-file or section mode, contained paths, and optional
destination tags.

## 3. Run the first synchronization

```sh
omnirepo sync
```

The run:

1. creates the durable run record
   (`<HOME>/.omnirepo/runs/<timestamp>-<id>.log`);
2. builds the source catalog and the per-repository plans;
3. applies the managed changes, runs the declared verification, and
   delivers one scoped commit per repository (`chore(omnirepo): sync
   managed content`) only after the checks pass;
4. repairs eligible failures with the configured adapters, bounded;
5. finalizes the record and exits with the stable code.

An unchanged repository creates no commit; an empty fleet is a success;
a missing or invalid machine authority is never scanned ambiently and
never triggers implicit migration.

## 4. What to expect

Routine success is quiet. `--output json` provides a versioned
machine-readable projection with the same outcomes. Every run leaves a
durable, timestamped record and ordinary Git history.

See [docs/breaks-inventory.md](breaks-inventory.md) for what changed
from earlier releases and [docs/breaking-guidance.md](breaking-guidance.md)
for the actionable migration guidance.

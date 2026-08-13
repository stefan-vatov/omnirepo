---
status: reference
scope: [distribution, package, public-api]
related:
  - fleet-lifecycle.md
---

# Product distribution

## Supported product surface

Omnirepo is a binary-only product. It does not provide a supported public Rust
library API. Removing the legacy `omnirepo_lib` exports is an intentional
breaking change and receives actionable migration guidance; no forbidden legacy
repository-orchestration authority remains available through a Rust API.

## Cargo package contents

The published Cargo package contains runtime essentials only: Rust source,
Cargo manifests and the lockfile, README, LICENSE, CHANGELOG, CONSTITUTION, and
runtime assets required by the binary. It excludes Beads data, agent skills,
Canon sources, CI workflows, generated packaging residue, and development-only
tooling.

The packaged crate, rather than only the worktree, must build and expose the
selected binary surface on the supported toolchain.

## Release identity and channels

A release is identified by one protected SemVer tag `vX.Y.Z` whose version
matches the Cargo package and whose immutable commit is the qualified release
source. A push to `main` never publishes. The exact qualified commit produces
both the crates.io package and the GitHub release with supported platform
binaries; artifact digests, version output, changelog, and migration guidance
must all identify that same commit and version.

Publication is idempotent and never moves, deletes, or repoints an existing tag
or public artifact. If one channel succeeds and another fails, the successful
immutable publication remains; retry first reconciles its identity and
publishes only the missing matching channel. Untrusted pull requests and
mutable external workflow authority cannot reach publication credentials.

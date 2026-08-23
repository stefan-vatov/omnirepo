---
status: reference
scope: [platforms, filesystems, paths, containment]
related:
  - configuration-authority.md
---

# Runtime platform support

## Supported platforms

Omnirepo supports Linux on all filesystems and macOS on APFS. Windows and
non-APFS macOS filesystems are unsupported and fail before synchronization
effects. Linux filesystem types are not allowlisted: local, network, virtual,
and userspace filesystems enter the same lifecycle. Each operation still fails
with a typed error when its target does not provide a required operating-system
capability, such as writable regular files, atomic same-directory rename,
durable synchronization, metadata preservation, or stable object identity.

## Path representation

Public configuration represents paths as UTF-8 strings with `/` separators.
Machine-configured authority roots may be absolute. Paths declared inside a
source or destination authority root are relative and must remain within that
root.

## Filesystem identity and containment

Configuration, source, destination, Git, process, and run-record operations use
one consistent platform-aware collision and containment identity. Absolute or
parent-traversing nested references, aliases that escape or duplicate an
authority identity, symlinks or mount crossings that widen authority, unsafe
hard-link targets, and non-regular managed objects fail closed at authority
boundaries.

## Resource exhaustion

The first constitutional release defines no product-specific numeric limits for
configuration, catalogs, managed content, source caches, staging, or temporary
storage. Operating-system allocation and storage failures are typed, durably
reported, and scoped to the affected shared authority or repository. Resource
pressure never truncates authoritative content, silently drops or reorders a
source, bypasses verification, or erases peer outcome accounting. Numeric input
budgets remain optional future hardening rather than a prerequisite for core
synchronization.

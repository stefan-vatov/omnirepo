# Private product module map

This document records the target topology for the binary product. It is a
design and move map, not a claim that every target path already exists.

## Product boundary

The root package remains exactly one Cargo-discovered private Rust binary
target, `omnirepo`. Cargo may discover that binary from the conventional
`src/main.rs` auto-target or from one explicit `[[bin]]` entry pointing to the
same file; the contract does not require explicit `[[bin]]` syntax. It does
not publish a library API. The two existing workspace tool crates,
`tools/omnirepo-dev` and `tools/omnirepo-test-support`, remain development-only
and publish-false. The product has no runtime path dependency on either crate.

`src/main.rs` is the thin private composition root. It declares the bounded
contexts and wires the executable. It does not become a public module export.

## Bounded contexts and ownership

| Context | Owns | Allowed product dependencies |
| --- | --- | --- |
| `configuration` | machine and repository configuration value rules | none |
| `lifecycle` | run records and fleet lifecycle composition | `configuration`, `managed_content`, `repository`, `source`, `platform` |
| `managed_content` | managed-file transaction behavior | `configuration` |
| `platform` | filesystem authority and platform boundary | none |
| `repository` | repository policy and state/delta proofs | `configuration`, `source` |
| `source` | source snapshot state and publication | `configuration` |

The arrows point from a consumer to a domain it may use. The lifecycle context
may consume the platform authority seam; platform consumes no product domain.
No context imports inward from an adapter, no context imports a later lifecycle
layer, and no catch-all `common`, `util`, `utils`, or `prelude` module is
introduced. Product modules stay private; no compatibility alias or legacy
library export returns.

## Tracer-bullet move order

1. Land and exercise the fixture contract in
   `tests/product_module_architecture_contract.rs`.
2. Move pure product domains and colocate their unit tests under the context
   roots: configuration, managed content, source, and repository.
3. Move platform authority and run-record code behind the platform/lifecycle
   seams. Keep security-sensitive filesystem behavior behind its authority
   boundary.
4. Update the composition root, package inventory, documentation inventory,
   and the live-root contract. Run the full workspace gates after the final
   move.

The contract's fixture tests provide a green intermediate product while these
moves happen. `validate_live_root` is the single reusable assertion seam: every
fixture test calls it, and the final move must call it for the repository root
from the integration gate. Until the move is complete, this file intentionally
describes the target rather than claiming that the current flat tree passes.

The composition root has an exact grammar. After comments and whitespace are
removed, `src/main.rs` must contain the six private external declarations in
the table order above, followed by one `fn main() { ... }` entry. The body is
the composition call surface. No `pub` or `cfg` attribute, inline module,
top-level `use`, constant, type, trait, impl, macro, or other top-level item is
accepted. A context module must therefore be file-backed and start at its
context directory; future CLI wiring belongs in the `main` body or in one of
the bounded contexts, not beside it.

The contract treats Rust source as a bounded, contained tree. It does not
follow symlinked files or directories, refuses source outside `src/`, and
reports traversal depth or file-count exhaustion. `syn` is the parsing
authority: parse failures, `Item::Verbatim`, item-position graph macros,
`include!`/`include_str!`/`include_bytes!`, `#[path]`, and `cfg_attr` fail
closed. Expression-position macros remain valid when their AST position does
not create a source edge; their token text is not searched for words such as
`include!`, so a literal string inside `format_args!` cannot create a false
positive. Structured `UseTree` imports are resolved separately from AST path
visits, so comments, strings, chars, raw strings, raw identifiers, aliases,
grouped imports, re-exports, globs, and `self`/`super` paths receive the same
dependency-edge rules. A glob rooted at `crate`, `self`, `super`, or a resolved
alias records the product context it names; an arbitrary external root such as
`serde::repository` records no product edge. Aliases are resolved to a fixed
point using the file's complete module path. Declared but unresolved aliases,
including relevant multi-hop cycles, fail closed; unknown external roots do
not fabricate product edges. A source reached through both runtime and exact
`cfg(test)` paths is rejected regardless of which traversal sees it first.
`#[cfg(test)]` attributes may be stacked across lines; external test-only
module descendants are recursively tolerated, while undeclared runtime files
remain errors. Every runtime child module remains private: `pub`, `pub(crate)`,
`pub(super)`, and other outward visibility forms are rejected; visibility on a
test-only module is handled with the test-only classification.

Cargo target, workspace, and package checks are structural rather than raw
substring checks. `toml_edit` is the manifest parsing authority, so malformed
or unsupported relevant shapes, duplicate keys, and duplicate tables fail
closed rather than allowing a later value to overwrite an earlier one. The
contract reads the relevant package/workspace/bin tables, accepts
single- or double-quoted arrays, handles multiline arrays and comments,
validates one `src/main.rs` binary by package name and path, requires the two
publish-false development members, and applies bounded Cargo-style
include/exclude globs to every runtime source file. Ordinary, build, dev, and
all target-specific dependency tables are inspected, as are
`workspace.dependencies` entries and `workspace = true` inheritance. A path
to either private tool is always rejected after lexical normalization,
including aliases and absolute forms. Paths that escape the product root are
rejected fail-closed. Each expected workspace member is opened through
no-follow path checks, and its manifest package name plus `publish = false`
identity are verified. `cargo_metadata` is authoritative for the resolved
workspace, targets, and effective dependency graph. `cargo package --list` is
authoritative for package contents; a package-list failure is a validation
failure, with no fallback inventory. Every reachable runtime source must be
listed, while every source reachable only through an exact `cfg(test)` module
graph must be absent from the package. The private-tool negative fixture uses
Cargo's recursive include semantics in an isolated package-list probe, verifies
that Cargo actually lists the tool source, and then requires the validator to
report forbidden development content; it does not pretend that Cargo's root
package includes nested workspace member packages. Dotted dependency tables,
including quoted dependency and target keys, are checked in ordinary, dev,
build, and target-specific forms. Grouped Rust use trees preserve the group
base for `self` aliases, so `use crate::repository::{self as repo};` still
records the repository edge.

Containment checks use no-follow metadata before reading the manifest, source
root, or source files. Source traversal reports directory-entry, metadata, and
read failures; it never drops an error through an iterator filter. Symlinked
roots, files, and module paths are rejected. Runtime inline modules are also
rejected because a text-only contract cannot prove their full edge set; an
inline module under `#[cfg(test)]` is classified as test-only and is excluded
from runtime reachability and dependency edges.

---
status: reference
scope: [synchronization, managed-content, partial-sections]
related:
  - configuration-authority.md
  - fleet-lifecycle.md
---

# Managed content

## Ownership boundaries

A managed file gives Omnirepo authority over the complete file. A managed
section gives it authority only over the content between its start and end
delimiters, expressed as comments valid for that file format.

For a file configured for partial management, existing paired delimiters bound
replacement. If the managed section is absent, Omnirepo appends the source
section together with both delimiters and leaves the file's existing local
content unchanged. The sync engine preserves all content outside an existing
managed section unchanged.

For the section being synchronized, its delimiters must resolve to exactly one
ordered, non-nested pair. Unmatched, reversed, duplicate, or nested delimiters
are ambiguous: synchronization fails and leaves the file unchanged.

Local edits inside a managed file or section are drift. Synchronization
replaces them without confirmation; this overwrite is expected behavior.

## Exact text

Omnirepo compares destination managed content with its authoritative source. A
match needs no content change; a difference is replaced with the source text.
It does not parse, normalize, reorder, or semantically merge configuration.

Omnirepo maintains a knowledge base of comment syntax for common configuration
formats solely to express managed-section delimiters. It otherwise treats
managed content as text.

## Failure boundary

Synchronization itself never crosses a managed section's delimiters. An
integration failure is reported for the fleet lifecycle to handle; semantic
repair is not silently folded into text synchronization.

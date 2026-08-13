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

Managed items and sections use explicit stable IDs made from ASCII lowercase
letters, digits, dots, underscores, and hyphens. IDs are exact and
case-sensitive. Duplicate IDs within one source are invalid. Compatible
cross-source overlap is resolved only by configured source order. Incompatible
whole-file/section collisions, ambiguous overlapping sections, and protected
authority-file targets fail before destination mutation. Multiple
non-overlapping named sections may share one destination file.

For a file configured for partial management, existing paired delimiters bound
replacement. If the managed section is absent, Omnirepo appends the source
section together with both delimiters and leaves the file's existing local
content unchanged. The sync engine preserves all content outside an existing
managed section unchanged.

For the section being synchronized, its delimiters must resolve to exactly one
ordered, non-nested pair. Unmatched, reversed, duplicate, or nested delimiters
are ambiguous: synchronization fails and leaves the file unchanged.

Partial sections use exact full-line named delimiters:

`<comment-token> omnirepo:start <section-id>`

`<comment-token> omnirepo:end <section-id>`

The built-in registry supplies line-comment tokens for common formats: `#`
for YAML, TOML, shell, Python, Ruby, and Make-style files; `//` for
JavaScript, TypeScript, Rust, and C-family files; `;` for INI-style files; and
`--` for SQL files. Unknown, extensionless, unnamed, whitespace-altered,
mismatched-ID, interleaved, nested, or payload-like marker cases are invalid.
User-defined and block-comment marker syntax is not supported.

When appending an absent section, Omnirepo inserts one separating newline,
preserves a detectable LF or CRLF style, and uses LF when no style is
detectable.

Local edits inside a managed file or section are drift. Synchronization
replaces them without confirmation; this overwrite is expected behavior.

## Exact text

Omnirepo compares destination managed content with its authoritative source as
opaque bytes. A byte-for-byte match needs no content change; a difference is
replaced with the source bytes. BOMs, invalid UTF-8, Unicode encodings, line
endings, and final-newline presence are authoritative. Omnirepo does not
decode, normalize, transcode, reorder, or semantically merge configuration.

Changed files are replaced atomically and durably through a same-directory
temporary file, file synchronization, rename, and parent-directory
synchronization. A failed or interrupted replacement exposes either the old
complete file or the new complete file, never partial managed content. Stale
temporary artifacts are recoverable and are not authority.

Existing files preserve ownership, mode, and supported ACLs and extended
attributes; inability to preserve them is a failure. Timestamps are not
preserved. New files use mode `0644` subject to the process umask. Read-only,
hard-linked, and non-regular targets fail before replacement. Safe contained
parent directories may be created; after failure, Omnirepo removes only empty
parents created by that operation. An unchanged target receives no filesystem
or metadata write.

Omnirepo maintains a knowledge base of comment syntax for common configuration
formats solely to recognize managed-section delimiter lines. Delimiter
recognition never changes payload bytes.

## Failure boundary

Synchronization itself never crosses a managed section's delimiters. An
integration failure is reported for the fleet lifecycle to handle; semantic
repair is not silently folded into text synchronization.

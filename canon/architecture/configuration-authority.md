---
status: reference
scope: [configuration, authority, precedence]
---

# Configuration authority

## Machine configuration

Machine configuration lives at `<HOME>/.omnirepo/config.<ext>`. It is the sole
authority for:

- the destination repositories in the fleet;
- the ordered source repositories that provide managed content; and
- source priority when sources provide overlapping managed content.

Source repositories contain authoritative managed content and source-side
configuration declaring the files and sections they provide. When sources
overlap, their configured order is the sole tiebreaker. Completion order,
content inspection, and inferred importance must not change precedence.

Machine configuration does not define destination-repository commands or the
managed scope inside a destination repository.

## Repository configuration

A destination repository may declare `.omnirepo.<ext>` at its root. That
configuration owns only that repository's applicability and commands: it may
select all managed content, allow selected content, exclude selected content,
or mix those controls.

When repository configuration exists, it is intentional and wholly governs
that repository's managed scope and commands. Inference must not broaden or
override it. Repository configuration cannot alter fleet membership,
authoritative sources, or source priority.

When repository configuration is absent, Omnirepo infers every applicable
source declaration it can match for that repository and synchronizes it. This
convention is a fallback, not an authority above explicit repository intent.

## Direction

Authority flows one way: from ordered source repositories to destination
repositories. Destination content is never learned from, merged back into, or
promoted to an authoritative source.

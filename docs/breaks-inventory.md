# User-visible breaks inventory

This inventory lists every user-visible break introduced by the
constitutional convergence and every misleading legacy text still
present in the surface.  Each break names the removed surface, the
replacement, and the guidance pointer; actionable guidance lives in
`docs/breaking-guidance.md` (owned by 2r9.14.4).

## Removed surfaces

| # | Removed surface | Replacement | Guidance |
|---|-----------------|-------------|----------|
| 1 | General multi-repository run surface (`run` with arbitrary repositories) | `sync` over the machine-declared fleet | `docs/breaking-guidance.md#multi-repo-run` |
| 2 | Tag-based clone workflow (legacy clone commands) | Source declarations with pinned revisions | `docs/breaking-guidance.md#tag-clone` |
| 3 | Ad hoc synchronization authority (arbitrary source/destination pairs) | Canonical machine configuration at `<HOME>/.omnirepo/config.yaml` | `docs/breaking-guidance.md#ad-hoc-sync` |
| 4 | Legacy config loader (0.8.x `Config`/`RepoConfig`/template authority) | The typed machine configuration authority | `docs/breaking-guidance.md#legacy-config` |
| 5 | General orchestrator / deployment / dependency surfaces | None (outside the constitutional purpose) | `docs/breaking-guidance.md#orchestrator` |
| 6 | Legacy logging, progress, table, and verbose output flags | Quiet human output + `--output json` | `docs/breaking-guidance.md#output` |

## Misleading legacy text still present

| # | Location | Misleading text | Action |
|---|----------|-----------------|--------|
| 1 | `omnirepo setup --help` / setup invocation | "setup is not available in this build" | The setup machinery exists; the CLI wiring lands with the setup command completion |
| 2 | `omnirepo validate --help` / validate invocation | "validate is not available in this build" | The validation surface lands with the validate command completion |
| 3 | `after_help` in `--help` | "Legacy general orchestration surfaces are unsupported and are not migrated automatically" | Lawful decline text; verify it never claims a migrating capability |

## Inventory contract

The inventory is enforced by `tests/legacy_surface_inventory.rs`: the
binary help must not claim the removed surfaces as available commands,
and this document must stay structured (one row per break with a
replacement and a guidance pointer).

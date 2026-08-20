# Test taxonomy and traceability

`tests/traceability/matrix.json` is the one canonical routing index for the
repository test system. It is test metadata, not product policy. It records
which test view owns each required behavior, constitutional clause, public
command, and lifecycle failure stage.

The matrix is an executable specification of ownership and evidence routing,
not proof that every listed product journey already runs. `implementation_status`
and `verification_status` make this boundary explicit:

- `specified` records a future contract. Its planned locators bind the owning
  implementation Bead and stable case/evidence identity. They do not name
  current executable proof.
- `implemented` requires a closed implementation Bead and an executable test
  locator. The file-backed validator resolves its path and selector.
- `verified` additionally requires an implemented row, executable fixture and
  evidence locators, a distinct closed downstream acceptance Bead, and exact
  structured evidence provenance in that Bead. Generic notes, comments, or
  acceptance prose are not proof.

The current matrix intentionally keeps product rows specified until their
implementation Beads supply their own proof. The traceability contract test
proves the matrix metadata and references; it does not masquerade as the 57
product journeys.

Validate it with:

```text
cargo test --locked --test traceability_contract -- --nocapture
```

The integration test reads the tracked `.beads/issues.jsonl` export. It does
not invoke `br`, `bv`, Git, a shell, a network, HOME, a clock, or an agent.
This keeps the validator deterministic and makes malformed fixtures easy to
replay.

## Matrix contract

The root schema is `omnirepo.traceability-matrix.v1`. The matrix declares the
complete taxonomy and the exact required sets for:

- constitutional principles, growth directives, boundaries, and tension-pair
  limits;
- public `sync`, `setup`, and `doctor` commands;
- lifecycle failure stages from invocation through cancellation and recovery;
- product behavior contracts such as authority, source materialization, exact
  whole-file and partial-section sync, verification, Git delivery, run
  records, repair causation, setup, validation, and packaging.

Each row has one stable row ID and one unique `reference`. It also has:

- one `primary_owner` Bead ID;
- deliberate `supporting_views` from unit, component, black-box E2E,
  adversarial, platform, scale, or optional views;
- a stable fixture identity, case ID, evidence ID, replay identity, and
  downstream acceptance Bead ID;
- an `implementation_bead` and explicit implementation/verification status;
- `test_locator` and `evidence_locator` objects whose role is status-aware;
- an `expected_effect` and a positive or negative observation;
- a negative case, so the boundary is tested as well as the happy path;
- `owner_decision_refs`, which may reference owner decisions but never select a
  value for one; and
- `constitutional_silence`, which must be explicit for optional or silent
  methods.

### Status-aware locators

Specified rows use this non-executable form:

```json
{"role":"planned","contract":"<implementation-bead>#<case-id>"}
```

The evidence contract uses the downstream Bead and evidence identity instead:

```json
{"role":"planned","contract":"<downstream-bead>#<evidence-id>"}
```

The validator checks both contracts. This makes a planned row uniquely
actionable without pretending that the canonical metadata test executes its
product case.

Implemented rows use `role: executable` for `test_locator`. The selector is an
exact Rust function or inline-module declaration. The validator lexes Rust and
ignores comments, strings, character literals, raw strings, and nested block
comments; identifier prefixes and incidental bytes do not resolve a selector.
It skips `macro_rules!` definitions, macro invocations with balanced brace,
parenthesis, or bracket token trees, and attribute token trees, so declarations
inside macro input never count as executable proof. Delimiter mismatches and
unsupported macro forms fail closed. Module paths use `::` components and must
match the declaration's containing module path.

Verified rows use `role: artifact` for `evidence_locator` and `role: fixture`
for the additional `fixture_locator`. These forms contain a repository-relative
path and a selector equal to the identity in a complete JSON record. Fixture
files must be exactly this schema:

```json
{"schema":"omnirepo.traceability-fixture.v1","row_id":"<row-id>","case_id":"<case-id>","fixture_id":"<fixture-id>","locator_role":"fixture","downstream_bead":"<downstream-bead>"}
```

Evidence files use the same exact shape with
`omnirepo.traceability-evidence.v1` and `evidence_id` in place of
`fixture_id`. The validator rejects YAML, extra fields, comments, arbitrary
bytes, and any identity mismatch. The closed downstream Bead must also carry
an exact `traceability_evidence` array record with the same row, case,
evidence, role, and downstream identities, plus normal close provenance. Pure
`validate_source` remains filesystem-free, so it validates shape and status
claims but never fabricates file evidence.

Case, evidence, and fixture identities are unique within the matrix. Owner
decision references must point to a closed Bead retaining both owner-decision
labels and close provenance; ordinary work Beads cannot be presented as owner
decisions.

The same Bead may own multiple different rows. A duplicate primary-owner
finding means that one required reference was assigned more than once, not
that a workstream cannot own several distinct behaviors.

## Projections and taxonomy

Constitutional and adversarial views are computed projections, not second
matrices. A constitutional reference must have constitutional kind, and every
row with a negative effect, adversarial test type, or adversarial supporting
view is consumed by the adversarial projection and must provide a non-empty
negative case. The validator checks these relationships on every row.

Optional and silence values are bidirectional. Every row must contain an
actual boolean `constitutional_silence` field. Optional or silent coverage
requires optional test type, `constitutional_silence: true`, and
`expected_effect: silence`. Required and conditional coverage must use a
non-optional test type, `constitutional_silence: false`, and a non-silence
effect. Missing, null, or one-sided edits therefore fail closed.

## Validator guarantees

The validator fails with bounded, replayable findings for:

- malformed, missing, unknown, duplicate, or wrong-typed schema fields;
- missing required clauses, commands, failure stages, or behaviors;
- duplicate row IDs or duplicate primary ownership of one reference;
- missing, stale, or orphaned Bead IDs in owners, decisions, and acceptance
  links;
- unsupported test types, views, effects, coverage status, or kind/reference
  combinations;
- future rows that claim executable locators, implemented rows without a
  closed implementation Bead, or verified rows without a distinct closed
  downstream acceptance Bead and exact structured evidence provenance;
- unresolved executable selectors, fixture locators, and evidence artifacts;
- missing fixtures, observations, negative cases, or stable evidence IDs; and
- policy-selecting fields or assignment-like values embedded in test data.

Policy assignment detection ignores whitespace around `=` and `:` and covers
selection spellings such as selected value, chosen value, effective value,
selection, and override. It rejects the assignment but never chooses its
value.

Matrix input is strict JSON, and the tracked Beads export is strict JSON Lines;
YAML, duplicate JSON keys, malformed records, oversized records, excessive
nesting, and unsafe file paths fail closed. Validator-safety bounds are
declared in the matrix root. They protect the validator from hostile or
accidental unbounded input and are not product synchronization limits;
product-specific numeric limits remain governed by their owning work and
decisions.

Findings retain a content-derived stable validator replay ID, a bounded UTF-8
path, and a bounded UTF-8 message. Row paths use the row's stable ID rather
than its array position, so inserting an unrelated row does not change replay
identity. The validator caps findings at 64 and reports truncation. It returns
data for every failure; it never repairs the matrix, chooses an owner value,
or rewrites a Bead.

## Adding a row

First identify the existing required reference or mark a genuinely optional
method with an `optional:` reference and explicit constitutional silence. Pick
one primary implementation/test Bead and link the downstream acceptance Bead.
Add a fixture identity and stable case/evidence IDs. Add a planned locator
contract bound to those IDs. Add supporting views only when they provide a
distinct assertion. Record owner decision references, not decision values. Run
the focused validator and the normal workspace gates.

When the owning work is complete, change status only with real executable,
fixture, and evidence locators and the corresponding closed Bead evidence.
Do not create a second constitutional or adversarial matrix. Those are views
of this matrix and must use its rows and ownership.

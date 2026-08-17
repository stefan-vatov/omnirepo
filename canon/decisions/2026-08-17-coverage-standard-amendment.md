---
status: reference
scope: [coverage]
---

# Decision: amend the coverage standard to the measured baseline

On 2026-08-17 the project owner chose to amend the declared coverage
thresholds instead of funding additional coverage work to meet the previous
standard. The active thresholds live in `canon/standards.md` (Coverage); this
record is immutable history.

The previous standard (90% lines, 80% functions, 80% regions, 95%
changed-line) exceeded the shipped suite's measured baseline on commit
`b57b96f`: 81.54% lines, 74.98% functions, 80.24% regions, and 83%
changed-line coverage. The amended thresholds sit below that baseline so the
gate binds without tripping on honest drift.

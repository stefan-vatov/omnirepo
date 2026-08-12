#!/usr/bin/env bash
set -euo pipefail

plan_tmp_dir="$(mktemp -d)"
trap 'rm -rf "$plan_tmp_dir"' EXIT

br ready --json >"$plan_tmp_dir/ready.json"

if ! command -v bv >/dev/null 2>&1; then
  jq '{
    status: "ok",
    plan: {
      tracks: [{
        track_id: "ready",
        items: map({id, title, priority, status})
      }],
      total_actionable: length,
      total_blocked: 0,
      summary: {
        source: "br ready",
        note: "Beads Viewer is unavailable; br ready remains authoritative."
      }
    }
  }' "$plan_tmp_dir/ready.json"
  exit 0
fi

bv --db .beads --robot-plan >"$plan_tmp_dir/viewer-plan.json"

jq --slurpfile ready "$plan_tmp_dir/ready.json" '
  ($ready[0] | map(.id)) as $ready_ids
  | .plan.tracks |= (
      map(.items |= map(select(.id as $id | $ready_ids | index($id))))
      | map(select(.items | length > 0))
    )
  | .plan.total_actionable = ([.plan.tracks[]?.items[]?] | length)
  | .plan.total_blocked = 0
  | .plan.summary = {
      source: "br ready",
      note: "Raw bv robot planning is advisory; this filtered plan is authoritative for autonomous work."
    }
' "$plan_tmp_dir/viewer-plan.json"

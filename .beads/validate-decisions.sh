#!/usr/bin/env bash
set -euo pipefail

validation_tmp_dir="$(mktemp -d)"
trap 'rm -rf "$validation_tmp_dir"' EXIT

jq -s '{issues: .}' .beads/issues.jsonl >"$validation_tmp_dir/all.json"

jq -r '.issues[] | select(.status != "closed") | select((.labels // []) | index("decision-needed")) | .id' \
  "$validation_tmp_dir/all.json" | sort >"$validation_tmp_dir/decision-label.ids"
jq -r '.issues[] | select(.status != "closed") | select((.labels // []) | index("human-input")) | .id' \
  "$validation_tmp_dir/all.json" | sort >"$validation_tmp_dir/human-label.ids"
jq -r '.issues[] | select(.status == "decision") | .id' \
  "$validation_tmp_dir/all.json" | sort >"$validation_tmp_dir/decision-status.ids"

cmp "$validation_tmp_dir/decision-label.ids" "$validation_tmp_dir/human-label.ids"
cmp "$validation_tmp_dir/decision-label.ids" "$validation_tmp_dir/decision-status.ids"

if command -v br >/dev/null 2>&1; then
  br ready --json >"$validation_tmp_dir/ready.json"
  br scheduler --json >"$validation_tmp_dir/scheduler.json"
  "$(dirname "$0")/agent-plan.sh" >"$validation_tmp_dir/agent-plan.json"

  if jq -e 'any(.[]; .status == "decision")' "$validation_tmp_dir/ready.json" >/dev/null; then
    echo "decision issue leaked into br ready" >&2
    exit 1
  fi

  if jq -e 'any(.recommendations[]?; .issue.status == "decision")' \
    "$validation_tmp_dir/scheduler.json" >/dev/null; then
    echo "decision issue leaked into br scheduler" >&2
    exit 1
  fi

  if jq -e '[.plan.tracks[]?.items[]? | select(.status == "decision")] | length > 0' \
    "$validation_tmp_dir/agent-plan.json" >/dev/null; then
    echo "decision issue leaked into the repository-owned agent plan" >&2
    exit 1
  fi

  if ! jq -e --slurpfile ready "$validation_tmp_dir/ready.json" '
    ($ready[0] | map(.id) | sort) as $ready_ids
    | ([.plan.tracks[]?.items[]?.id] | sort) == $ready_ids
  ' "$validation_tmp_dir/agent-plan.json" >/dev/null; then
    echo "repository-owned agent plan differs from br ready" >&2
    exit 1
  fi
fi

echo "decision workflow is consistent"

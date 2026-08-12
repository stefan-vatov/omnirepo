#!/usr/bin/env bash
set -euo pipefail

validation_tmp_dir="$(mktemp -d)"
trap 'rm -rf "$validation_tmp_dir"' EXIT

br list --status all --json >"$validation_tmp_dir/all.json"
br ready --json >"$validation_tmp_dir/ready.json"
br scheduler --json >"$validation_tmp_dir/scheduler.json"

jq -r '.issues[] | select((.labels // []) | index("decision-needed")) | .id' \
  "$validation_tmp_dir/all.json" | sort >"$validation_tmp_dir/decision-label.ids"
jq -r '.issues[] | select((.labels // []) | index("human-input")) | .id' \
  "$validation_tmp_dir/all.json" | sort >"$validation_tmp_dir/human-label.ids"
jq -r '.issues[] | select(.status == "decision") | .id' \
  "$validation_tmp_dir/all.json" | sort >"$validation_tmp_dir/decision-status.ids"

cmp "$validation_tmp_dir/decision-label.ids" "$validation_tmp_dir/human-label.ids"
cmp "$validation_tmp_dir/decision-label.ids" "$validation_tmp_dir/decision-status.ids"

if jq -e 'any(.[]; .status == "decision")' "$validation_tmp_dir/ready.json" >/dev/null; then
  echo "decision issue leaked into br ready" >&2
  exit 1
fi

if jq -e 'any(.recommendations[]?; .issue.status == "decision")' \
  "$validation_tmp_dir/scheduler.json" >/dev/null; then
  echo "decision issue leaked into br scheduler" >&2
  exit 1
fi

echo "decision workflow is consistent"

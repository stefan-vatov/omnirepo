#!/usr/bin/env bash

set -euo pipefail

expected="${1:?expected capability class is required}"
output="$(mktemp)"
trap 'rm -f -- "$output"' EXIT

if ! cargo test --bin omnirepo --all-targets --locked -- --nocapture >"$output" 2>&1; then
    cat "$output" >&2
    exit 1
fi
cat "$output"
grep -F -- "authority-capability: exercised-supported=${expected}" "$output" >/dev/null || {
    printf 'authority-capability: required class was not exercised: %s\n' "$expected" >&2
    exit 1
}

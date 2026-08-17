#!/usr/bin/env bash

set -euo pipefail

readonly RUST_TOOLCHAIN="1.86.0"
readonly CARGO_LLVM_COV_VERSION="0.8.7"
readonly COVERAGE_LINES_MIN=90
readonly COVERAGE_FUNCTIONS_MIN=80
readonly COVERAGE_REGIONS_MIN=80

if [[ ! -f Cargo.toml ]]; then
    printf 'coverage: run this entry point from the repository root\n' >&2
    exit 2
fi

readonly coverage_dir="$PWD/coverage"
readonly cargo_llvm_cov=(scripts/cargo-1.86 llvm-cov)
readonly coverage_ownership_matrix="$PWD/tests/traceability/matrix.json"
readonly coverage_ownership_map="$PWD/tests/traceability/coverage-ownership.json"
readonly coverage_ownership_manifest="tools/omnirepo-dev/Cargo.toml"
readonly coverage_ownership_command=(scripts/cargo-1.86 run --quiet --locked --manifest-path "$coverage_ownership_manifest" -- coverage-ownership --repo-root "$PWD" --lcov "$coverage_dir/lcov.info" --matrix "$coverage_ownership_matrix" --ownership "$coverage_ownership_map" --json)
readonly coverage_changed_command=(scripts/cargo-1.86 run --quiet --locked --manifest-path "$coverage_ownership_manifest" -- changed-coverage --repo-root "$PWD" --lcov "$coverage_dir/lcov.info" --base "${OMNIREPO_COVERAGE_BASE:-}" --json)

printf 'coverage: rust-toolchain=%s cargo-llvm-cov=%s lines>=%s functions>=%s regions>=%s\n' \
    "$RUST_TOOLCHAIN" \
    "$CARGO_LLVM_COV_VERSION" \
    "$COVERAGE_LINES_MIN" \
    "$COVERAGE_FUNCTIONS_MIN" \
    "$COVERAGE_REGIONS_MIN"

if tool_version_output="$("${cargo_llvm_cov[@]}" --version)"; then
    :
else
    tool_status=$?
    printf 'coverage: unable to run cargo-llvm-cov %s with Rust %s\n' \
        "$CARGO_LLVM_COV_VERSION" "$RUST_TOOLCHAIN" >&2
    exit "$tool_status"
fi

case "$tool_version_output" in
    "cargo-llvm-cov ${CARGO_LLVM_COV_VERSION}"*)
        ;;
    *)
        printf 'coverage: expected cargo-llvm-cov %s, found: %s\n' \
            "$CARGO_LLVM_COV_VERSION" "$tool_version_output" >&2
        exit 2
        ;;
esac

mkdir -p "$coverage_dir"
readonly coverage_profile_parent="$coverage_dir/.profiles"
mkdir -p "$coverage_profile_parent"
readonly coverage_profile_root="$(mktemp -d "$coverage_profile_parent/run.XXXXXXXXXX")"
readonly coverage_profile_file="$coverage_profile_root/%m-%p.profraw"
coverage_interrupted=0

coverage_mark_interrupted() {
    coverage_interrupted=1
    exit 130
}

coverage_cleanup() {
    local status=$?
    if (( coverage_interrupted )); then
        printf 'coverage: interrupted; profile root retained at %s\n' \
            "$coverage_profile_root" >&2
    else
        rm -rf -- "$coverage_profile_root"
    fi
    exit "$status"
}

trap coverage_cleanup EXIT
trap coverage_mark_interrupted HUP INT TERM

# cargo-llvm-cov uses this directory for instrumented build output, raw
# profiles, and merged report inputs.  The pattern is also present for child
# tools that replace their environment, so they cannot fall back to a CWD
# default profile.
export CARGO_LLVM_COV_TARGET_DIR="$coverage_profile_root"
export LLVM_PROFILE_FILE="$coverage_profile_file"

"${cargo_llvm_cov[@]}" clean --workspace
"${cargo_llvm_cov[@]}" --workspace --all-targets --all-features --locked --no-report

# Keep the threshold result while continuing to emit every diagnostic report.
set +e
"${cargo_llvm_cov[@]}" report --summary-only \
    --fail-under-lines "$COVERAGE_LINES_MIN" \
    --fail-under-functions "$COVERAGE_FUNCTIONS_MIN" \
    --fail-under-regions "$COVERAGE_REGIONS_MIN" \
    | tee "$coverage_dir/summary.txt"
summary_pipeline_status=("${PIPESTATUS[@]}")

"${cargo_llvm_cov[@]}" report --lcov --output-path "$coverage_dir/lcov.info"
lcov_status=$?
"${cargo_llvm_cov[@]}" report --html --output-dir "$coverage_dir"
html_status=$?
"${coverage_ownership_command[@]}" > "$coverage_dir/ownership.json"
ownership_status=$?
"${coverage_changed_command[@]}" > "$coverage_dir/changed-coverage.json"
changed_status=$?
set -e

summary_status="${summary_pipeline_status[0]}"
summary_output_status="${summary_pipeline_status[1]}"
final_status=0
if (( summary_status != 0 )); then
    final_status="$summary_status"
elif (( summary_output_status != 0 )); then
    final_status="$summary_output_status"
elif (( lcov_status != 0 )); then
    final_status="$lcov_status"
elif (( html_status != 0 )); then
    final_status="$html_status"
elif (( ownership_status != 0 )); then
    final_status="$ownership_status"
elif (( changed_status != 0 )); then
    final_status="$changed_status"
fi

printf 'coverage: statuses summary=%s summary-output=%s lcov=%s html=%s ownership=%s changed=%s final=%s\n' \
    "$summary_status" \
    "$summary_output_status" \
    "$lcov_status" \
    "$html_status" \
    "$ownership_status" \
    "$changed_status" \
    "$final_status"
exit "$final_status"

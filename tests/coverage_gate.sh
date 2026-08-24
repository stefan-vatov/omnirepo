#!/usr/bin/env bash

set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly repository_root
readonly coverage_script="$repository_root/scripts/coverage.sh"
test_root="$(mktemp -d)"
readonly test_root
trap 'rm -rf -- "$test_root"' EXIT

git -C "$repository_root" check-ignore -q coverage/summary.txt

# The gate fixtures intercept the toolchain through their exported `cargo`
# function; the override makes coverage.sh use it instead of the repo shim.
export OMNIREPO_COVERAGE_CARGO=cargo

fake_cargo() {
    if [[ -n "${FAKE_CARGO_LOG:-}" ]]; then
        printf '%s\n' "$*" >> "$FAKE_CARGO_LOG"
    fi

    if [[ "${1:-}" == "+1.95.0" ]]; then
        shift
    fi
    if [[ "${1:-}" == "run" ]]; then
        local after_dash=0
        local subcommand=""
        for arg in "$@"; do
            if (( after_dash )); then
                subcommand="$arg"
                break
            fi
            if [[ "$arg" == "--" ]]; then
                after_dash=1
            fi
        done
        if [[ "$subcommand" == "changed-coverage" ]]; then
            printf '%s' '{"schema":"omnirepo.changed-executable-coverage.v1","base":"b","head":"h","threshold_percent":80,"executable_changed_lines":1,"covered_changed_lines":1,"coverage_percent":100,"coverage_ratio":"1/1","passed":true,"lines":[{"path":"src/main.rs","line":1,"status":"covered"}]}'
            return "${FAKE_CHANGED_STATUS:-0}"
        fi
        printf '{"schema":"omnirepo.coverage-ownership-report.v1","sources":[]}'
        return "${FAKE_OWNERSHIP_STATUS:-0}"
    fi
    [[ "${1:-}" == "llvm-cov" ]] || return 90
    if [[ -n "${FAKE_PROFILE_LOG:-}" ]]; then
        printf 'target=%s\nprofile=%s\n' \
            "${CARGO_LLVM_COV_TARGET_DIR-<unset>}" \
            "${LLVM_PROFILE_FILE-<unset>}" >> "$FAKE_PROFILE_LOG"
    fi
    shift

    case "${1:-}" in
        --version)
            printf 'cargo-llvm-cov %s\n' "${FAKE_TOOL_VERSION:-0.8.7}"
            return 0
            ;;
        clean)
            return "${FAKE_CLEAN_STATUS:-0}"
            ;;
        --workspace)
            if [[ " $* " == *" --no-report "* ]]; then
                if [[ -n "${FAKE_TEST_FIFO:-}" ]]; then
                    printf '%s\n' "$$" >> "$FAKE_TEST_READY"
                    read -r < "$FAKE_TEST_FIFO"
                fi
                if [[ -n "${LLVM_PROFILE_FILE+x}" ]]; then
                    local profile_path="${LLVM_PROFILE_FILE//%m/fake-module}"
                    profile_path="${profile_path//%p/$$}"
                    mkdir -p "$(dirname "$profile_path")"
                    printf 'fake profile\n' > "$profile_path"
                else
                    printf 'default root profile\n' > "$PWD/default_fake.profraw"
                    mkdir -p "$PWD/tools/omnirepo-dev"
                    printf 'default tool profile\n' > \
                        "$PWD/tools/omnirepo-dev/default_fake.profraw"
                fi
                return "${FAKE_TEST_STATUS:-0}"
            fi
            return 91
            ;;
        report)
            shift
            case "${1:-}" in
                --summary-only)
                    printf 'fake summary\n'
                    return "${FAKE_SUMMARY_STATUS:-0}"
                    ;;
                --lcov)
                    while (($# > 0)); do
                        if [[ "$1" == "--output-path" ]]; then
                            printf 'fake lcov\n' > "$2"
                            break
                        fi
                        shift
                    done
                    return "${FAKE_LCOV_STATUS:-0}"
                    ;;
                --html)
                    while (($# > 0)); do
                        if [[ "$1" == "--output-dir" ]]; then
                            mkdir -p "$2"
                            printf 'fake html\n' > "$2/index.html"
                            break
                        fi
                        shift
                    done
                    return "${FAKE_HTML_STATUS:-0}"
                    ;;
            esac
            return 92
            ;;
    esac
    return 93
}
export -f fake_cargo
function cargo() {
    fake_cargo "$@"
}
export -f cargo

function tee() {
    local output_path="${!#}"
    cat > "$output_path"
    return "${FAKE_TEE_STATUS:-0}"
}
export -f tee

assert_status() {
    local expected="$1"
    local actual="$2"
    if [[ "$actual" != "$expected" ]]; then
        printf 'coverage-gate-test: expected exit %s, got %s\n' "$expected" "$actual" >&2
        return 1
    fi
}

run_gate() {
    local case_name="$1"
    local expected_status="$2"
    local case_dir="$test_root/$case_name"
    mkdir -p "$case_dir"
    : > "$case_dir/Cargo.toml"
    export FAKE_CARGO_LOG="$case_dir/cargo.log"
    export FAKE_PROFILE_LOG="$case_dir/profile.log"
    if [[ "$case_name" != tool-version-mismatch ]]; then
        mkdir -p "$case_dir/coverage/.profiles/peer-run" "$case_dir/tools/omnirepo-dev"
        printf 'peer profile\n' > "$case_dir/coverage/.profiles/peer-run/peer.profraw"
        printf 'peer root profile\n' > "$case_dir/peer.profraw"
        printf 'peer tool profile\n' > "$case_dir/tools/omnirepo-dev/peer.profraw"
    fi

    set +e
    (cd "$case_dir" && "$coverage_script") > "$case_dir/output.log" 2>&1
    local actual_status=$?
    set -e

    printf 'coverage-gate-test: case=%s expected=%s actual=%s\n' \
        "$case_name" "$expected_status" "$actual_status"
    assert_status "$expected_status" "$actual_status"
    if [[ "$case_name" != tool-version-mismatch ]]; then
        [[ -s "$case_dir/coverage/.profiles/peer-run/peer.profraw" ]]
        [[ -s "$case_dir/peer.profraw" ]]
        [[ -s "$case_dir/tools/omnirepo-dev/peer.profraw" ]]
        [[ ! -e "$case_dir/default_fake.profraw" ]]
        [[ ! -e "$case_dir/tools/omnirepo-dev/default_fake.profraw" ]]
        [[ -s "$case_dir/profile.log" ]]
        grep -E -- "^target=$case_dir/coverage/\\.profiles/run\\." \
            "$case_dir/profile.log" >/dev/null
        grep -E -- "^profile=$case_dir/coverage/\\.profiles/run\\." \
            "$case_dir/profile.log" >/dev/null
        [[ -z "$(find "$case_dir/coverage/.profiles" -mindepth 1 -maxdepth 1 -type d -name 'run.*' -print)" ]]
    fi
}

export FAKE_TOOL_VERSION=0.8.7
export FAKE_CLEAN_STATUS=0
export FAKE_TEST_STATUS=0
export FAKE_SUMMARY_STATUS=0
export FAKE_TEE_STATUS=0
export FAKE_LCOV_STATUS=0
export FAKE_HTML_STATUS=0
export FAKE_OWNERSHIP_STATUS=0
export FAKE_CHANGED_STATUS=0
unset FAKE_TEST_FIFO FAKE_TEST_READY
run_gate all-success 0
[[ -s "$test_root/all-success/coverage/summary.txt" ]]
[[ -s "$test_root/all-success/coverage/lcov.info" ]]
[[ -s "$test_root/all-success/coverage/index.html" ]]
[[ -s "$test_root/all-success/coverage/ownership.json" ]]
[[ -s "$test_root/all-success/coverage/changed-coverage.json" ]]
grep -F -- 'summary=0 summary-output=0 lcov=0 html=0 ownership=0 changed=0 final=0' \
    "$test_root/all-success/output.log" >/dev/null

export FAKE_SUMMARY_STATUS=5
export FAKE_TEE_STATUS=6
export FAKE_LCOV_STATUS=7
export FAKE_HTML_STATUS=8
export FAKE_OWNERSHIP_STATUS=9
export FAKE_CHANGED_STATUS=11
run_gate simultaneous-failure-precedence 5
grep -F -- 'summary=5 summary-output=6 lcov=7 html=8 ownership=9 changed=11 final=5' \
    "$test_root/simultaneous-failure-precedence/output.log" >/dev/null

export FAKE_SUMMARY_STATUS=1
export FAKE_TEE_STATUS=0
export FAKE_LCOV_STATUS=0
export FAKE_HTML_STATUS=0
export FAKE_OWNERSHIP_STATUS=0
export FAKE_CHANGED_STATUS=0
run_gate threshold-failure 1
[[ -s "$test_root/threshold-failure/coverage/summary.txt" ]]
[[ -s "$test_root/threshold-failure/coverage/lcov.info" ]]
[[ -s "$test_root/threshold-failure/coverage/index.html" ]]
[[ -s "$test_root/threshold-failure/coverage/ownership.json" ]]
grep -F -- 'llvm-cov --workspace --all-targets --all-features --locked --no-report' \
    "$test_root/threshold-failure/cargo.log" >/dev/null
grep -F -- 'llvm-cov report --summary-only --fail-under-lines 80 --fail-under-functions 73 --fail-under-regions 78' \
    "$test_root/threshold-failure/cargo.log" >/dev/null
grep -F -- 'run --quiet --locked --manifest-path tools/omnirepo-dev/Cargo.toml -- coverage-ownership' \
    "$test_root/threshold-failure/cargo.log" >/dev/null
grep -F -- 'run --quiet --locked --manifest-path tools/omnirepo-dev/Cargo.toml -- changed-coverage' \
    "$test_root/threshold-failure/cargo.log" >/dev/null

export FAKE_SUMMARY_STATUS=0
export FAKE_TEE_STATUS=8
export FAKE_LCOV_STATUS=0
export FAKE_HTML_STATUS=0
run_gate summary-output-failure 8
[[ -s "$test_root/summary-output-failure/coverage/lcov.info" ]]

export FAKE_TEE_STATUS=0
export FAKE_LCOV_STATUS=7
export FAKE_HTML_STATUS=0
run_gate lcov-failure 7
[[ -s "$test_root/lcov-failure/coverage/index.html" ]]

export FAKE_LCOV_STATUS=0
export FAKE_LCOV_STATUS=0
export FAKE_OWNERSHIP_STATUS=6
run_gate ownership-failure 6
[[ -s "$test_root/ownership-failure/coverage/lcov.info" ]]

export FAKE_OWNERSHIP_STATUS=0
export FAKE_CHANGED_STATUS=4
run_gate changed-failure 4
[[ -s "$test_root/changed-failure/coverage/ownership.json" ]]
[[ -s "$test_root/changed-failure/coverage/changed-coverage.json" ]]
grep -F -- 'summary=0 summary-output=0 lcov=0 html=0 ownership=0 changed=4 final=4' \
    "$test_root/changed-failure/output.log" >/dev/null
export FAKE_CHANGED_STATUS=0

export FAKE_TEST_STATUS=9
run_gate test-failure 9
[[ ! -e "$test_root/test-failure/coverage/summary.txt" ]]

export FAKE_TEST_STATUS=0
export FAKE_OWNERSHIP_STATUS=0
export FAKE_TOOL_VERSION=0.8.6
run_gate tool-version-mismatch 2
[[ ! -e "$test_root/tool-version-mismatch/coverage" ]]

export FAKE_TOOL_VERSION=0.8.7
interrupted_root="$test_root/interrupted"
mkdir -p "$interrupted_root"
: > "$interrupted_root/Cargo.toml"
mkfifo "$interrupted_root/release.fifo"
export FAKE_CARGO_LOG="$interrupted_root/cargo.log"
export FAKE_PROFILE_LOG="$interrupted_root/profile.log"
export FAKE_TEST_FIFO="$interrupted_root/release.fifo"
export FAKE_TEST_READY="$interrupted_root/ready"
set +e
(cd "$interrupted_root" && "$coverage_script") > "$interrupted_root/output.log" 2>&1 &
interrupted_pid=$!
for _ in {1..100000}; do
    if [[ -s "$interrupted_root/ready" ]]; then
        break
    fi
done
[[ -s "$interrupted_root/ready" ]]
kill -TERM "$interrupted_pid"
wait "$interrupted_pid"
interrupted_status=$?
set -e
[[ "$interrupted_status" != 0 ]]
grep -F -- 'profile root retained at ' "$interrupted_root/output.log" >/dev/null
retained_profile_root="$(sed -n 's/^coverage: interrupted; profile root retained at //p' "$interrupted_root/output.log")"
[[ -d "$retained_profile_root" ]]
[[ "$retained_profile_root" == "$interrupted_root/coverage/.profiles/run."* ]]
unset FAKE_TEST_FIFO FAKE_TEST_READY

concurrent_root="$test_root/concurrent"
mkdir -p "$concurrent_root/coverage/.profiles/peer-run" "$concurrent_root/tools/omnirepo-dev"
: > "$concurrent_root/Cargo.toml"
printf 'peer profile\n' > "$concurrent_root/coverage/.profiles/peer-run/peer.profraw"
mkfifo "$concurrent_root/release.fifo"
export FAKE_TOOL_VERSION=0.8.7
export FAKE_CLEAN_STATUS=0
export FAKE_TEST_STATUS=0
export FAKE_SUMMARY_STATUS=0
export FAKE_TEE_STATUS=0
export FAKE_LCOV_STATUS=0
export FAKE_HTML_STATUS=0
export FAKE_OWNERSHIP_STATUS=0
export FAKE_CARGO_LOG="$concurrent_root/cargo.log"
export FAKE_PROFILE_LOG="$concurrent_root/profile.log"
export FAKE_TEST_FIFO="$concurrent_root/release.fifo"
export FAKE_TEST_READY="$concurrent_root/ready"
set +e
(cd "$concurrent_root" && "$coverage_script") > "$concurrent_root/first.log" 2>&1 &
concurrent_first_pid=$!
(cd "$concurrent_root" && "$coverage_script") > "$concurrent_root/second.log" 2>&1 &
concurrent_second_pid=$!
for _ in {1..100000}; do
    if [[ -s "$concurrent_root/ready" ]] && [[ "$(wc -l < "$concurrent_root/ready")" -ge 2 ]]; then
        break
    fi
done
[[ -s "$concurrent_root/ready" ]] && [[ "$(wc -l < "$concurrent_root/ready")" -ge 2 ]]
{ echo; echo; } > "$concurrent_root/release.fifo"
wait "$concurrent_first_pid"
concurrent_first_status=$?
wait "$concurrent_second_pid"
concurrent_second_status=$?
set -e
[[ "$concurrent_first_status" == 0 ]]
[[ "$concurrent_second_status" == 0 ]]
[[ -s "$concurrent_root/coverage/.profiles/peer-run/peer.profraw" ]]
concurrent_profile_root_count="$(sed -n 's#^target=\(.*\/run\.[^/]*\)$#\1#p' \
    "$concurrent_root/profile.log" | sort -u | wc -l)"
[[ "$concurrent_profile_root_count" == 2 ]]
[[ -z "$(find "$concurrent_root/coverage/.profiles" -mindepth 1 -maxdepth 1 -type d -name 'run.*' -print)" ]]
unset FAKE_TEST_FIFO FAKE_TEST_READY

printf 'coverage-gate-test: all status, report, tool-version, and locked-argument checks passed\n'

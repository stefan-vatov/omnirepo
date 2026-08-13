#!/usr/bin/env bash

# Static contract strings intentionally contain shell expansion syntax.
# shellcheck disable=SC2016
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly repository_root
contract_script="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
readonly contract_script
contract_root="${1:-${COVERAGE_CONTRACT_ROOT:-$repository_root}}"
readonly contract_root
readonly coverage_script="$contract_root/scripts/coverage.sh"
readonly workflow="$contract_root/.github/workflows/coverage.yml"
readonly readme="$contract_root/README.md"
readonly cargo_config="$contract_root/.cargo/config.toml"
readonly gitignore="$contract_root/.gitignore"
readonly cargo_manifest="$contract_root/Cargo.toml"
readonly authority_capability="$contract_root/tests/authority_capability.sh"
readonly expected_profile="--profile coverage --json"
test_root="$(mktemp -d)"
readonly test_root
trap 'rm -rf -- "$test_root"' EXIT

fail_contract() {
    printf 'coverage-contract: RED: %s\n' "$1" >&2
    exit 1
}

line_number() {
    local pattern="$1"
    local file="$2"
    rg -n -- "$pattern" "$file" | cut -d: -f1 | head -n 1 || true
}

last_line_number() {
    local pattern="$1"
    local file="$2"
    rg -n -- "$pattern" "$file" | cut -d: -f1 | tail -n 1 || true
}

assert_gate_contract() {
    [[ -x "$coverage_script" ]] || fail_contract "coverage gate is not executable: $coverage_script"
    [[ -x "$authority_capability" ]] || fail_contract "authority capability probe is not executable: $authority_capability"

    for expected in \
        'readonly RUST_TOOLCHAIN="1.86.0"' \
        'readonly CARGO_LLVM_COV_VERSION="0.8.7"' \
        'readonly COVERAGE_LINES_MIN=90' \
        'readonly COVERAGE_FUNCTIONS_MIN=80' \
        'readonly COVERAGE_REGIONS_MIN=80'; do
        grep -Fx -- "$expected" "$coverage_script" >/dev/null || \
            fail_contract "coverage gate changed its required authority: $expected"
    done

    grep -F -- '"${cargo_llvm_cov[@]}" --workspace --all-targets --all-features --locked --no-report' \
        "$coverage_script" >/dev/null || \
        fail_contract "coverage gate does not measure the locked workspace with all targets and features"

    for expected in \
        '--fail-under-lines "$COVERAGE_LINES_MIN"' \
        '--fail-under-functions "$COVERAGE_FUNCTIONS_MIN"' \
        '--fail-under-regions "$COVERAGE_REGIONS_MIN"'; do
        grep -F -- "$expected" "$coverage_script" >/dev/null || \
            fail_contract "coverage gate is missing threshold enforcement: $expected"
    done

    for expected in \
        'readonly coverage_ownership_command=' \
        'coverage-ownership --repo-root' \
        'coverage_ownership_command[@]' \
        'ownership_status=$?' \
        'elif (( ownership_status != 0 )); then'; do
        grep -F -- "$expected" "$coverage_script" >/dev/null || \
            fail_contract "coverage gate is missing deterministic ownership attribution: $expected"
    done

    for expected in \
        'readonly coverage_profile_parent="$coverage_dir/.profiles"' \
        'readonly coverage_profile_root="$(mktemp -d "$coverage_profile_parent/run.XXXXXXXXXX")"' \
        'readonly coverage_profile_file="$coverage_profile_root/%m-%p.profraw"' \
        'export CARGO_LLVM_COV_TARGET_DIR="$coverage_profile_root"' \
        'export LLVM_PROFILE_FILE="$coverage_profile_file"' \
        'trap coverage_cleanup EXIT' \
        'trap coverage_mark_interrupted HUP INT TERM' \
        '    rm -rf -- "$coverage_profile_root"'; do
        grep -F -- "$expected" "$coverage_script" >/dev/null || \
            fail_contract "coverage gate is missing owned profile isolation: $expected"
    done
    if rg -n -- 'rm[[:space:]]+-rf[[:space:]]+--[[:space:]]+"\$coverage_dir"' "$coverage_script"; then
        fail_contract "coverage gate deletes the shared report directory"
    fi
    grep -F -- 'trap coverage_cleanup EXIT' "$coverage_script" >/dev/null || \
        fail_contract "owned profile cleanup is not registered for report completion"
    grep -F -- 'profile root retained at' "$coverage_script" >/dev/null || \
        fail_contract "interrupted coverage runs are not discoverable"

    summary_line="$(line_number 'report --summary-only' "$coverage_script")"
    lcov_line="$(line_number 'report --lcov' "$coverage_script")"
    html_line="$(line_number 'report --html' "$coverage_script")"
    [[ -n "$summary_line" && -n "$lcov_line" && -n "$html_line" ]] || \
        fail_contract "coverage gate does not produce summary, LCOV, and HTML diagnostics"
    (( summary_line < lcov_line && lcov_line < html_line )) || \
        fail_contract "coverage diagnostics are not ordered after the threshold summary"

    grep -Fx -- 'set +e' "$coverage_script" >/dev/null || \
        fail_contract "coverage gate does not preserve report failures while collecting diagnostics"
    grep -Fx -- 'summary_pipeline_status=("${PIPESTATUS[@]}")' "$coverage_script" >/dev/null || \
        fail_contract "coverage gate does not capture both threshold and tee statuses"
    for expected in \
        'summary_status="${summary_pipeline_status[0]}"' \
        'summary_output_status="${summary_pipeline_status[1]}"' \
        'final_status=0' \
        'lcov_status=$?' \
        'html_status=$?' \
        'exit "$final_status"'; do
        grep -Fx -- "$expected" "$coverage_script" >/dev/null || \
            fail_contract "coverage gate does not preserve the report status: $expected"
    done
    restored_set_e_line="$(last_line_number '^set -e$' "$coverage_script")"
    [[ -n "$restored_set_e_line" && "$restored_set_e_line" -gt "$summary_line" ]] || \
        fail_contract "coverage gate does not restore errexit after diagnostic reports"
    if rg -n -- '\|\|[[:space:]]*true' "$coverage_script"; then
        fail_contract "coverage gate masks a failure with || true"
    fi

    if rg -n -e '(^|[[:space:];|])(source|\.|cat|grep|awk|sed)[[:space:]]+.*coverage' \
        -e '<[[:space:]]*.*coverage_dir' "$coverage_script"; then
        fail_contract "generated coverage reports are used as gate configuration"
    fi
}

assert_single_authority() {
    for surface in "$workflow" "$readme" "$cargo_config" "$contract_root/Cargo.toml"; do
        [[ -f "$surface" ]] || continue
        if rg -n -- \
            'COVERAGE_(LINES|FUNCTIONS|REGIONS)_MIN|--fail-under-(lines|functions|regions)' \
            "$surface"; then
            fail_contract "mutable coverage threshold authority exists outside scripts/coverage.sh: $surface"
        fi
    done

    while IFS= read -r surface; do
        [[ "$surface" == "$coverage_script" ]] && continue
        if rg -n -- 'cargo[[:space:]]+llvm-cov|llvm-cov[[:space:]]+report|COVERAGE_(LINES|FUNCTIONS|REGIONS)_MIN' \
            "$surface"; then
            fail_contract "alternate mutable coverage authority exists in $surface"
        fi
    done < <(rg --files "$contract_root/scripts" 2>/dev/null || true)

    local cargo_surfaces=("$cargo_config")
    [[ -f "$contract_root/Cargo.toml" ]] && cargo_surfaces+=("$contract_root/Cargo.toml")
    if rg -n -- '^[[:space:]]*(coverage|coverage-html)[[:space:]]*=' "${cargo_surfaces[@]}"; then
        fail_contract "Cargo configuration contains an alternate coverage alias"
    fi
}

assert_invocations() {
    grep -F -- "$expected_profile" "$readme" >/dev/null || \
        fail_contract "README does not document the manifest-owned coverage profile"
    grep -F -- "$expected_profile" "$workflow" >/dev/null || \
        fail_contract "CI does not invoke the manifest-owned coverage profile"

    if rg -n -e 'cargo \+1\.86\.0 (coverage|coverage-html|llvm-cov)' "$readme" "$workflow"; then
        fail_contract "legacy coverage command remains outside scripts/coverage.sh"
    fi
    if rg -n -e 'cargo llvm-cov' "$workflow"; then
        fail_contract "CI contains duplicate llvm-cov gate logic"
    fi
}

assert_workflow_contract() {
    local gate_line
    local upload_line

    gate_line="$(line_number '--profile coverage --json' "$workflow")"
    upload_line="$(line_number 'uses: actions/upload-artifact@v7\.0\.1' "$workflow")"
    [[ -n "$gate_line" && -n "$upload_line" && "$gate_line" -lt "$upload_line" ]] || \
        fail_contract "coverage upload is not after the gate invocation"
    grep -Fx -- '        if: always()' "$workflow" >/dev/null || \
        fail_contract "coverage upload is not configured with if: always()"
    if grep -n -- 'continue-on-error:' "$workflow"; then
        fail_contract "coverage gate or upload can mask a failed job"
    fi
    grep -Fx -- '          if-no-files-found: error' "$workflow" >/dev/null || \
        fail_contract "coverage upload does not fail when reports are absent"

    grep -Fx -- '  contents: read' "$workflow" >/dev/null || \
        fail_contract "coverage workflow permissions are not bounded to contents: read"
    for action in \
        'uses: actions/checkout@v7.0.1' \
        'uses: dtolnay/rust-toolchain@1.86.0' \
        'uses: Swatinem/rust-cache@v2.9.2' \
        'uses: taiki-e/install-action@v2.85.11' \
        'uses: actions/upload-artifact@v7.0.1'; do
        grep -Fx -- "        $action" "$workflow" >/dev/null || \
            fail_contract "unbounded or changed action version: $action"
    done
    if rg -n -- 'uses: .*@(main|master|stable|latest)([[:space:]]|$)' "$workflow"; then
        fail_contract "workflow uses an unbounded action reference"
    fi
}

assert_authority_platform_contract() {
    grep -F -- 'platform-authority:' "$workflow" >/dev/null || \
        fail_contract "CI does not run the required authority platform capability job"
    grep -F -- 'ubuntu-latest' "$workflow" >/dev/null || \
        fail_contract "authority platform job does not cover Linux"
    grep -F -- 'macos-latest' "$workflow" >/dev/null || \
        fail_contract "authority platform job does not cover macOS"
    grep -F -- 'linux-ext-family' "$workflow" >/dev/null || \
        fail_contract "Linux authority job does not assert ext-family classification"
    grep -F -- 'macos-apfs' "$workflow" >/dev/null || \
        fail_contract "macOS authority job does not assert APFS classification"
    grep -F -- 'authority_capability.sh' "$workflow" >/dev/null || \
        fail_contract "authority platform job does not run the visible capability probe"
    grep -F -- 'authority-capability: exercised-supported=' "$contract_root/tests/authority_capability.sh" >/dev/null || \
        fail_contract "capability probe does not require visible exercised classification"
}

assert_artifact_boundaries() {
    [[ -f "$gitignore" ]] || fail_contract "repository .gitignore is missing"
    grep -Fx -- '*.profraw' "$gitignore" >/dev/null || \
        fail_contract "LLVM raw-profile residue is not ignored with a narrow basename pattern"
    [[ -f "$cargo_manifest" ]] || fail_contract "repository Cargo.toml is missing"
    grep -Fx -- '    "coverage",' "$cargo_manifest" >/dev/null || \
        fail_contract "coverage/ is not explicitly excluded from the product package"

    if [[ "$contract_root" == "$repository_root" ]]; then
        for residue in \
            'default_16337534979066748888_0_2206979.profraw' \
            'nested/llvm/default_16337534979066748888_0_2207191.profraw'; do
            git -C "$repository_root" check-ignore -q --no-index -- "$residue" || \
                fail_contract "raw-profile residue is not ignored at every depth: $residue"
        done
        git -C "$repository_root" check-ignore -q --no-index -- coverage/summary.txt || \
            fail_contract "generated coverage reports are not ignored"
    fi
}

run_status_fixtures() {
    [[ "${COVERAGE_CONTRACT_SKIP_STATUS_FIXTURES:-0}" == 1 ]] && return 0
    [[ -x "$contract_root/tests/coverage_gate.sh" ]] || \
        fail_contract "status fixture is missing from contract root"
    printf 'coverage-contract: threshold fixture (local command)\n'
    bash "$contract_root/tests/coverage_gate.sh"
    printf 'coverage-contract: threshold fixture (CI command)\n'
    bash "$contract_root/tests/coverage_gate.sh"
}

run_fixture_contract() {
    local fixture_root="$1"
    local expected_status="$2"
    local label="$3"
    local output
    local actual_status

    if output="$(COVERAGE_CONTRACT_SKIP_MUTATIONS=1 COVERAGE_CONTRACT_SKIP_STATUS_FIXTURES=1 \
        bash "$contract_script" "$fixture_root" 2>&1)"; then
        actual_status=0
    else
        actual_status=$?
    fi
    printf 'coverage-contract: mutation=%s status=%s\n%s\n' \
        "$label" "$actual_status" "$output"
    [[ "$actual_status" == "$expected_status" ]] || \
        fail_contract "mutation fixture $label returned $actual_status; expected $expected_status"
    if [[ "$expected_status" != 0 ]]; then
        grep -F -- 'coverage-contract: RED:' <<<"$output" >/dev/null || \
            fail_contract "mutation fixture $label did not produce a RED contract result"
    else
        grep -F -- 'coverage-contract: GREEN:' <<<"$output" >/dev/null || \
            fail_contract "mutation fixture $label did not produce a GREEN contract result"
    fi
}

run_mutation_fixtures() {
    [[ "${COVERAGE_CONTRACT_SKIP_MUTATIONS:-0}" == 1 ]] && return 0

    local fixture_root="$test_root/coverage-contract-fixture"
    mkdir -p "$fixture_root/scripts" "$fixture_root/.github/workflows" "$fixture_root/.cargo"

    restore_canonical_fixture() {
        cp "$coverage_script" "$fixture_root/scripts/coverage.sh"
        cp "$workflow" "$fixture_root/.github/workflows/coverage.yml"
        cp "$readme" "$fixture_root/README.md"
        cp "$cargo_config" "$fixture_root/.cargo/config.toml"
        cp "$gitignore" "$fixture_root/.gitignore"
        cp "$cargo_manifest" "$fixture_root/Cargo.toml"
        mkdir -p "$fixture_root/tests"
        cp "$authority_capability" "$fixture_root/tests/authority_capability.sh"
    }

    restore_canonical_fixture

    sed -i 's/readonly COVERAGE_LINES_MIN=90/readonly COVERAGE_LINES_MIN=89/' \
        "$fixture_root/scripts/coverage.sh"
    run_fixture_contract "$fixture_root" 1 threshold-mutated-red

    restore_canonical_fixture
    sed -i '0,/--all-targets/s//--all-targets-omitted/' "$fixture_root/scripts/coverage.sh"
    run_fixture_contract "$fixture_root" 1 target-scope-mutated-red

    restore_canonical_fixture
    sed -i '0,/--locked --no-report/s//--no-report/' "$fixture_root/scripts/coverage.sh"
    run_fixture_contract "$fixture_root" 1 locked-dependency-mutated-red

    restore_canonical_fixture
    sed -i 's/summary_pipeline_status=("${PIPESTATUS\[@\]}")/summary_pipeline_status=(0 0)/' \
        "$fixture_root/scripts/coverage.sh"
    run_fixture_contract "$fixture_root" 1 status-capture-mutated-red

    restore_canonical_fixture
    printf '\ncoverage = "alternate-authority"\n' >> "$fixture_root/.cargo/config.toml"
    run_fixture_contract "$fixture_root" 1 alternate-authority-mutated-red

    restore_canonical_fixture
    sed -i '/^\*\.profraw$/d' "$fixture_root/.gitignore"
    run_fixture_contract "$fixture_root" 1 raw-profile-ignore-mutated-red

    restore_canonical_fixture
    sed -i '/^    "coverage",$/d' "$fixture_root/Cargo.toml"
    run_fixture_contract "$fixture_root" 1 package-coverage-exclusion-mutated-red

    restore_canonical_fixture
    sed -i '/^final_status=0$/d' "$fixture_root/scripts/coverage.sh"
    run_fixture_contract "$fixture_root" 1 all-success-final-status-mutated-red

    restore_canonical_fixture
    sed -i '/coverage_ownership_command/d' "$fixture_root/scripts/coverage.sh"
    run_fixture_contract "$fixture_root" 1 ownership-attribution-mutated-red

    restore_canonical_fixture
    sed -i '/export CARGO_LLVM_COV_TARGET_DIR=/d' "$fixture_root/scripts/coverage.sh"
    run_fixture_contract "$fixture_root" 1 profile-root-propagation-mutated-red

    restore_canonical_fixture
    run_fixture_contract "$fixture_root" 0 canonical-restored-green
}

run_status_fixtures
assert_gate_contract
assert_invocations
assert_workflow_contract
assert_authority_platform_contract
assert_single_authority
assert_artifact_boundaries

run_mutation_fixtures

printf 'coverage-contract: GREEN: local and CI invoke manifest-owned coverage profile; locked scope, threshold status, report retention, and single authority pass\n'

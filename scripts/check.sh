#!/usr/bin/env bash
# The gate: everything that must pass before a change is claimed done.
#
# The same four checks CLAUDE.md and README.md have always named, in one command
# so there is no set to remember and no chance of running three of them. Nothing
# here is new policy — the point is that "did you run the gate" has one answer.
#
# The bash twin of `check.ps1`; keep the two in step.
#
# Fixture-dependent suites are run with REVIEW_REQUIRE_FIXTURES=1, so a missing
# asset or an un-vendored dependency fails loudly instead of being skipped.
# Skipping is right for a fresh checkout and wrong here: without it a green run
# means "nothing was found to run" just as readily as "everything passed". Pass
# --allow-skips for a checkout that genuinely lacks the fixtures.
set -uo pipefail

allow_skips=0
no_shader_check=0
for arg in "$@"; do
    case "$arg" in
        --allow-skips) allow_skips=1 ;;
        --no-shader-check) no_shader_check=1 ;;
        *) echo "unknown option: $arg" >&2; exit 2 ;;
    esac
done

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$script_dir/.."

failures=()

step() {
    local name="$1"; shift
    echo
    echo "=== $name ==="
    if ! "$@"; then
        failures+=("$name")
        echo "$name FAILED" >&2
    fi
}

step 'cargo fmt --check' cargo fmt --all -- --check
step 'cargo clippy' cargo clippy --workspace --all-targets -- -D warnings

if [ "$allow_skips" -eq 0 ]; then
    export REVIEW_REQUIRE_FIXTURES=1
fi
step 'cargo test' cargo test --workspace
unset REVIEW_REQUIRE_FIXTURES

if [ "$no_shader_check" -eq 0 ]; then
    step 'shader bytecode' "$script_dir/../packaging/check-shader-bytecode.sh"
fi

echo
if [ "${#failures[@]}" -gt 0 ]; then
    printf 'FAILED: %s\n' "$(IFS=', '; echo "${failures[*]}")" >&2
    exit 1
fi
echo "All checks passed."

#!/usr/bin/env bash
# Run all synthetic scenario tests using the fixture harness.
# Requires: firstaid binary built with `cargo build --release`
set -euo pipefail

BINARY="${1:-./target/release/firstaid}"
FIXTURES_DIR="fixtures/scenarios"
PASS=0
FAIL=0

echo "Running synthetic scenario tests..."
echo ""

for fixture in "$FIXTURES_DIR"/*.json; do
    scenario=$(basename "$fixture" .json)
    echo -n "  Testing: $scenario ... "

    # Run in test mode with the fixture
    output=$("$BINARY" --fixture "$fixture" diagnose --json 2>&1)
    exit_code=$?

    if [ $exit_code -eq 0 ]; then
        echo "PASS"
        PASS=$((PASS + 1))
    else
        echo "FAIL (exit $exit_code)"
        echo "    Output: $output"
        FAIL=$((FAIL + 1))
    fi
done

echo ""
echo "Results: $PASS passed, $FAIL failed"
if [ $FAIL -gt 0 ]; then
    exit 1
fi
echo "All scenario tests passed."

#!/bin/bash
# Fake workload that writes throughput data in the saturator protocol format.
# Useful for testing the external workload pipeline without a real application.
#
# Usage: fake_workload.sh [ops_per_sec]
# Reads SATURATOR_THROUGHPUT_FILE env var for output path.

set -euo pipefail

THROUGHPUT_FILE="${SATURATOR_THROUGHPUT_FILE:?SATURATOR_THROUGHPUT_FILE env var must be set}"
OPS=${1:-10000}

> "$THROUGHPUT_FILE"

while true; do
    # Add some noise (+/- 5%)
    noise=$(( (RANDOM % 1000) - 500 ))
    actual=$(( OPS + OPS * noise / 10000 ))
    ts=$(date +%s%3N)
    echo "$ts $actual" >> "$THROUGHPUT_FILE"
    sleep 1
done

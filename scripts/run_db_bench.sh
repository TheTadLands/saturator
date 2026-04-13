#!/bin/bash
# Wrapper for RocksDB's db_bench that outputs throughput in saturator's protocol format.
#
# Protocol: each line is "<timestamp_ms> <cumulative_ops>"
# The SATURATOR_THROUGHPUT_FILE env var specifies the output file path.
#
# Usage: run_db_bench.sh [db_bench args...]
#
# Example:
#   SATURATOR_THROUGHPUT_FILE=/tmp/tp.txt ./run_db_bench.sh \
#     --benchmarks=readrandom --num=10000000 --threads=4 --db=/tmp/rocksdb_bench

set -euo pipefail

THROUGHPUT_FILE="${SATURATOR_THROUGHPUT_FILE:?SATURATOR_THROUGHPUT_FILE env var must be set}"
REPORT_FILE="/tmp/saturator_db_bench_report_$$.csv"

# Clear the throughput file
> "$THROUGHPUT_FILE"

# Wipe the db and equalize page-cache state so every sample starts from the
# same point. Without this, compaction state and page-cache residency from the
# prior sample bleed into the next one and make iobw/memcap sweeps noisy.
DB_PATH=""
for arg in "$@"; do
    case "$arg" in
        --db=*) DB_PATH="${arg#--db=}" ;;
    esac
done
if [ -n "$DB_PATH" ]; then
    rm -rf "$DB_PATH"
fi
sync
echo 3 > /proc/sys/vm/drop_caches 2>/dev/null || true

# Capture start time once. Each report row's timestamp is derived from
# start_ms + secs_elapsed * 1000, guaranteeing exactly 1s spacing regardless
# of when the wrapper polls.
START_MS=$(date +%s%3N)

# Run db_bench with --report_interval_seconds=1 which writes aggregate QPS
# directly to a file (bypassing pipe buffering). We convert interval QPS to
# cumulative ops for saturator's protocol.
db_bench --report_interval_seconds=1 --report_file="$REPORT_FILE" "$@" > /dev/null 2>&1 &
DB_BENCH_PID=$!

# Poll the report file, convert interval QPS rows to cumulative ops, and
# append to the throughput file. Each report row is: secs_elapsed,interval_qps
last_lines=0

while kill -0 "$DB_BENCH_PID" 2>/dev/null; do
    sleep 0.5
    [ -f "$REPORT_FILE" ] || continue

    # Count data lines (excluding header)
    curr_lines=$(tail -n +2 "$REPORT_FILE" 2>/dev/null | wc -l)
    [ "$curr_lines" -gt "$last_lines" ] || continue

    # Recompute all cumulative values and emit only the new ones.
    # Timestamp = start_ms + secs_elapsed * 1000 (from column 1).
    awk -F, -v skip="$last_lines" -v start="$START_MS" '
        NR == 1 { next }
        { cum += $2; n++ }
        n > skip {
            ts = start + $1 * 1000
            print ts " " cum
        }
    ' "$REPORT_FILE" >> "$THROUGHPUT_FILE"

    last_lines=$curr_lines
done

# Process any remaining lines after db_bench exits
if [ -f "$REPORT_FILE" ]; then
    curr_lines=$(tail -n +2 "$REPORT_FILE" 2>/dev/null | wc -l)
    if [ "$curr_lines" -gt "$last_lines" ]; then
        awk -F, -v skip="$last_lines" -v start="$START_MS" '
            NR == 1 { next }
            { cum += $2; n++ }
            n > skip {
                ts = start + $1 * 1000
                print ts " " cum
            }
        ' "$REPORT_FILE" >> "$THROUGHPUT_FILE"
    fi
fi

wait "$DB_BENCH_PID" 2>/dev/null || true
rm -f "$REPORT_FILE"

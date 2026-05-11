#!/bin/bash
# Wrapper for RocksDB's db_bench that outputs throughput in saturator's protocol format.
#
# Throughput protocol: each line is "<timestamp_ms> <cumulative_ops>"
# The SATURATOR_THROUGHPUT_FILE env var specifies the output file path.
#
# Stats protocol (optional): "<timestamp_ms>,<metric_name>,<value>" per row.
# The SATURATOR_STATS_FILE env var specifies the output file path. When set,
# the wrapper enables db_bench's periodic stats dump, parses the output, and
# emits compaction/stall time-series into the stats file. When unset, the
# stats dump is not requested and db_bench stdout is discarded.
#
# Usage: run_db_bench.sh [db_bench args...]
#
# Example:
#   SATURATOR_THROUGHPUT_FILE=/tmp/tp.txt ./run_db_bench.sh \
#     --benchmarks=readrandom --num=10000000 --threads=4 --db=/tmp/rocksdb_bench

set -euo pipefail

THROUGHPUT_FILE="${SATURATOR_THROUGHPUT_FILE:?SATURATOR_THROUGHPUT_FILE env var must be set}"
STATS_FILE="${SATURATOR_STATS_FILE:-}"
REPORT_FILE="/tmp/saturator_db_bench_report_$$.csv"
STATS_RAW="/tmp/saturator_db_bench_raw_$$.log"

# Clear the throughput file (and stats file if stats are enabled)
> "$THROUGHPUT_FILE"
if [ -n "$STATS_FILE" ]; then
    > "$STATS_FILE"
fi

# Extract --db path from arguments.
DB_PATH=""
for arg in "$@"; do
    case "$arg" in
        --db=*) DB_PATH="${arg#--db=}" ;;
    esac
done

# Optional pre-fill: SATURATOR_PREFILL holds the full db_bench argument string
# for the fill phase (e.g. "--benchmarks=fillrandom --num=1000000
# --value_size=4096 --threads=1 --db=/tmp/rocksdb_rw"). On the first
# invocation the fill runs and the result is snapshotted. Subsequent
# invocations restore the snapshot via cp instead of re-running the fill,
# guaranteeing byte-identical DB state per sample.
PREFILL_SNAPSHOT=""
if [ -n "${SATURATOR_PREFILL:-}" ] && [ -n "$DB_PATH" ]; then
    PREFILL_SNAPSHOT="${DB_PATH}_prefill_snapshot"
    if [ ! -d "$PREFILL_SNAPSHOT" ]; then
        rm -rf "$DB_PATH"
        # shellcheck disable=SC2086
        db_bench $SATURATOR_PREFILL > /dev/null 2>&1
        cp -r "$DB_PATH" "$PREFILL_SNAPSHOT"
    fi
    if [ "${SATURATOR_PREFILL_ONLY:-}" = "1" ]; then
        exit 0
    fi
    rm -rf "$DB_PATH"
    cp -r "$PREFILL_SNAPSHOT" "$DB_PATH"
elif [ -n "$DB_PATH" ]; then
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
#
# When stats are enabled, also request periodic stats dumps and capture
# stdout to a raw log that we parse after db_bench exits. The `Uptime(secs):`
# field in each dump anchors per-interval timestamps to START_MS, so we get
# time-series rows without having to tail the log concurrently.
STATS_ARGS=()
if [ -n "$STATS_FILE" ]; then
    STATS_ARGS=(--stats_interval_seconds=1 --stats_per_interval=1 --statistics)
    db_bench "${STATS_ARGS[@]}" --report_interval_seconds=1 --report_file="$REPORT_FILE" "$@" > "$STATS_RAW" 2>&1 &
else
    db_bench --report_interval_seconds=1 --report_file="$REPORT_FILE" "$@" > /dev/null 2>&1 &
fi
DB_BENCH_PID=$!

# Ensure db_bench dies if we (the wrapper) get signaled. sh does not
# propagate signals to backgrounded children by default, so without this
# trap saturator's SIGTERM kills the wrapper but leaves db_bench orphaned,
# reparented to init, and still writing to the shared throughput file —
# corrupting downstream samples.
parse_stats() {
    [ -n "$STATS_FILE" ] && [ -f "$STATS_RAW" ] || return 0
    awk -v start="$START_MS" '
        function emit(metric, val,   ts) {
            if (uptime < 0) return
            ts = start + uptime * 1000
            printf "%d,%s,%s\n", ts, metric, val
        }
        BEGIN { section = ""; uptime = -1; emitted_comp = 0 }

        /^\*\* DB Stats \*\*/ { section = "db"; uptime = -1; emitted_comp = 0; next }
        /^\*\* Compaction Stats/ { section = "comp"; next }
        /^\*\*/ { section = ""; next }

        section == "db" && /^Uptime\(secs\):/ { uptime = $2 + 0; next }

        section == "db" && /^Cumulative stall:/ {
            if (match($0, /[0-9.]+ percent/)) {
                val = substr($0, RSTART, RLENGTH - 8)
                emit("cumulative_stall_pct", val)
            }
            next
        }
        section == "db" && /^Interval stall:/ {
            if (match($0, /[0-9.]+ percent/)) {
                val = substr($0, RSTART, RLENGTH - 8)
                emit("interval_stall_pct", val)
            }
            next
        }

        # Sum row fields (whitespace-split). Size = 2 tokens ("NN.NN KB").
        # RocksDB 9.x adds WPreComp(GB) after Write(GB), shifting later cols:
        #   $6 Read(GB)  $9 Write(GB)  $13 W-Amp  $16 Comp(sec)  $18 Comp(cnt)
        section == "comp" && !emitted_comp && $1 == "Sum" {
            if ($4 == "KB" || $4 == "MB" || $4 == "GB" || $4 == "TB" || $4 == "B") {
                emit("cumulative_compact_read_gb",  $6)
                emit("cumulative_compact_write_gb", $9)
                emit("write_amp",                   $13)
                emit("cumulative_compact_sec",      $16)
                emit("cumulative_compact_count",    $18)
            }
            emitted_comp = 1
            next
        }
    ' "$STATS_RAW" >> "$STATS_FILE"
}

cleanup() {
    kill -TERM "$DB_BENCH_PID" 2>/dev/null || true
    for _ in 1 2 3 4; do
        kill -0 "$DB_BENCH_PID" 2>/dev/null || break
        sleep 0.25
    done
    kill -KILL "$DB_BENCH_PID" 2>/dev/null || true
    wait "$DB_BENCH_PID" 2>/dev/null || true
    parse_stats
    rm -f "$REPORT_FILE" "$STATS_RAW"
}
trap 'cleanup; exit 130' INT TERM HUP
trap cleanup EXIT

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
parse_stats
rm -f "$REPORT_FILE" "$STATS_RAW"

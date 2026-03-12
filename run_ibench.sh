#!/bin/bash
#
# iBench experiment harness — runs N copies of an iBench SOI and collects
# cgroup system metrics, producing CSV compatible with plot_results.py.
#
# The cpu SOI normally uses OpenMP to sweep 1..maxThreads internally.
# We set OMP_NUM_THREADS=1 to force single-threaded mode and spawn N
# separate processes instead — matching the saturator's process-based model.
#
# Usage: ./run_experiment.sh <soi> [options]
#
# SOIs: cpu, l1d, l2, l3, memBw, memCap, l1i
#
# Options:
#   --max-workers N    Max worker count (default: num_cpus * 4)
#   --duration N       Measurement duration per data point in seconds (default: 5)
#   --samples N        Samples per data point (default: 5)
#   --step N           Worker count increment (default: 1)
#   --warmup N         Warmup duration in seconds (default: 1)
#   --output DIR       Output directory (default: /app/output)

set -euo pipefail

# Force cpu SOI to single-threaded mode so we control parallelism via process count
export OMP_NUM_THREADS=1

# ── Parse arguments ──────────────────────────────────────────────────────────

SOI=""
MAX_WORKERS=""
DURATION=5
SAMPLES=5
STEP=1
WARMUP=1
OUTPUT_DIR="/app/output"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --max-workers) MAX_WORKERS="$2"; shift 2 ;;
        --duration)    DURATION="$2";    shift 2 ;;
        --samples)     SAMPLES="$2";     shift 2 ;;
        --step)        STEP="$2";        shift 2 ;;
        --warmup)      WARMUP="$2";      shift 2 ;;
        --output)      OUTPUT_DIR="$2";  shift 2 ;;
        -*)            echo "Unknown option: $1"; exit 1 ;;
        *)
            if [[ -z "$SOI" ]]; then
                SOI="$1"; shift
            else
                echo "Unexpected argument: $1"; exit 1
            fi
            ;;
    esac
done

if [[ -z "$SOI" ]]; then
    echo "Usage: $0 <soi> [--max-workers N] [--duration N] [--samples N] [--step N] [--warmup N]"
    echo "SOIs: cpu, l1d, l2, l3, memBw, memCap, l1i"
    exit 1
fi

SOI_BIN="/ibench/src/${SOI}"
if [[ ! -x "$SOI_BIN" ]]; then
    echo "Error: SOI binary not found: $SOI_BIN"
    exit 1
fi

# Detect number of CPUs (prefer cpuset, fall back to nproc)
if [[ -f /sys/fs/cgroup/cpuset.cpus.effective ]]; then
    # Parse ranges like "4-7" or "0,2,4-6" into a count
    NUM_CPUS=0
    IFS=',' read -ra PARTS < /sys/fs/cgroup/cpuset.cpus.effective
    for part in "${PARTS[@]}"; do
        if [[ "$part" == *-* ]]; then
            lo="${part%-*}"; hi="${part#*-}"
            NUM_CPUS=$(( NUM_CPUS + hi - lo + 1 ))
        else
            NUM_CPUS=$(( NUM_CPUS + 1 ))
        fi
    done
else
    NUM_CPUS=$(nproc)
fi
if [[ -z "$MAX_WORKERS" ]]; then
    MAX_WORKERS=$((NUM_CPUS * 4))
fi

echo "=== iBench experiment: $SOI ==="
echo "CPUs: $NUM_CPUS, max workers: $MAX_WORKERS, duration: ${DURATION}s, samples: $SAMPLES, step: $STEP, warmup: ${WARMUP}s"
if [[ "$SOI" == "cpu" ]]; then
    echo "Note: OMP_NUM_THREADS=1 (single-threaded per process)"
fi

# ── Cgroup metric helpers ────────────────────────────────────────────────────

read_cpu_usage_usec() {
    awk '/^usage_usec/ {print $2}' /sys/fs/cgroup/cpu.stat 2>/dev/null || echo 0
}

read_cpu_system_usec() {
    awk '/^system_usec/ {print $2}' /sys/fs/cgroup/cpu.stat 2>/dev/null || echo 0
}

read_psi_total() {
    local file="$1" kind="$2"
    grep "^${kind}" "$file" 2>/dev/null | grep -oP 'total=\K[0-9]+' || echo 0
}

take_snapshot() {
    local cpu_u sys_u psi_c psi_is psi_if wall_n
    cpu_u=$(read_cpu_usage_usec)
    sys_u=$(read_cpu_system_usec)
    psi_c=$(read_psi_total /sys/fs/cgroup/cpu.pressure "some")
    psi_is=$(read_psi_total /sys/fs/cgroup/io.pressure "some")
    psi_if=$(read_psi_total /sys/fs/cgroup/io.pressure "full")
    wall_n=$(date +%s%N)
    echo "${cpu_u},${sys_u},${psi_c},${psi_is},${psi_if},${wall_n}"
}

compute_metrics() {
    local before="$1" after="$2"
    IFS=',' read -r b_cpu b_sys b_psi_c b_psi_is b_psi_if b_wall <<< "$before"
    IFS=',' read -r a_cpu a_sys a_psi_c a_psi_is a_psi_if a_wall <<< "$after"

    local d_cpu=$(( a_cpu - b_cpu ))
    local d_sys=$(( a_sys - b_sys ))
    local d_wall_ns=$(( a_wall - b_wall ))
    local d_wall_us=$(( d_wall_ns / 1000 ))
    local total_avail_us=$(( d_wall_us * NUM_CPUS ))

    local cpu_pct="0.00" sys_pct="0.00"
    if [[ "$total_avail_us" -gt 0 ]]; then
        cpu_pct=$(printf "%.2f" "$(echo "scale=4; $d_cpu * 100 / $total_avail_us" | bc)")
        sys_pct=$(printf "%.2f" "$(echo "scale=4; $d_sys * 100 / $total_avail_us" | bc)")
    fi

    local d_psi_cpu=$(( a_psi_c - b_psi_c ))
    local d_psi_io_some=$(( a_psi_is - b_psi_is ))
    local d_psi_io_full=$(( a_psi_if - b_psi_if ))

    local io_psi_pct="0.00"
    if [[ "$d_wall_us" -gt 0 ]]; then
        io_psi_pct=$(printf "%.2f" "$(echo "scale=4; $d_psi_io_some * 100 / $d_wall_us" | bc)")
    fi

    echo "${cpu_pct},${sys_pct},0.00,${io_psi_pct},${d_psi_cpu},${d_psi_io_some},${d_psi_io_full}"
}

# ── Median helper ────────────────────────────────────────────────────────────

median() {
    echo "$1" | sort -g | awk '{a[NR]=$1} END{print a[int((NR+1)/2)]}'
}

# ── Run one measurement ──────────────────────────────────────────────────────

run_measurement() {
    local n_workers="$1"
    local total_time=$((WARMUP + DURATION))
    local pids=()

    # cpu SOI with OMP_NUM_THREADS=1: runs 1 thread for total_time seconds
    # (the internal sweep loop runs once since maxThreads=1 with OMP_NUM_THREADS=1...
    #  but actually omp_get_num_procs() ignores OMP_NUM_THREADS. The loop will still
    #  iterate 1..maxThreads but each phase uses only 1 thread. Duration per phase =
    #  total_time/maxThreads which may be short. Give it extra time.)
    local soi_duration="$total_time"
    if [[ "$SOI" == "cpu" ]]; then
        # cpu.cpp divides duration by omp_get_num_procs() for per-phase time.
        # With OMP_NUM_THREADS=1, each phase still uses 1 thread but there are
        # NUM_CPUS phases. Multiply duration so total wall time >= warmup + duration.
        soi_duration=$(( total_time * NUM_CPUS ))
    fi

    local soi_args=("$soi_duration")
    if [[ "$SOI" == "l1i" ]]; then
        soi_args=("$total_time" "20")
    fi

    for ((i = 0; i < n_workers; i++)); do
        "$SOI_BIN" "${soi_args[@]}" >/dev/null 2>&1 &
        pids+=($!)
    done

    sleep "$WARMUP"
    local snap_before
    snap_before=$(take_snapshot)
    sleep "$DURATION"
    local snap_after
    snap_after=$(take_snapshot)

    # Kill workers (they may still be running through later phases)
    for pid in "${pids[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    for pid in "${pids[@]}"; do
        wait "$pid" 2>/dev/null || true
    done

    compute_metrics "$snap_before" "$snap_after"
}

# ── Main experiment loop ─────────────────────────────────────────────────────

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RUN_DIR="${OUTPUT_DIR}/ibench_${SOI}_${TIMESTAMP}"
mkdir -p "$RUN_DIR"

CSV_FILE="${RUN_DIR}/ibench_${SOI}_throughput_vs_workers.csv"
echo "workers,cpu_ops_sec,io_ops_sec,cpu_ops_stddev,io_ops_stddev,throughput_per_worker,cpu_pct,system_pct,io_util_pct,io_psi_pct,psi_cpu_some_us,psi_io_some_us,psi_io_full_us" > "$CSV_FILE"

echo ""
echo "Output: $CSV_FILE"
echo ""

for ((n = 1; n <= MAX_WORKERS; n += STEP)); do
    echo -n "Workers: $n/$MAX_WORKERS  "

    cpu_pcts="" sys_pcts="" io_psi_pcts=""
    psi_cpu_vals="" psi_io_some_vals="" psi_io_full_vals=""

    for ((s = 1; s <= SAMPLES; s++)); do
        echo -n "."
        metrics=$(run_measurement "$n")

        IFS=',' read -r m_cpu m_sys m_io_util m_io_psi m_psi_cpu m_psi_io_some m_psi_io_full <<< "$metrics"

        cpu_pcts="${cpu_pcts}${m_cpu}\n"
        sys_pcts="${sys_pcts}${m_sys}\n"
        io_psi_pcts="${io_psi_pcts}${m_io_psi}\n"
        psi_cpu_vals="${psi_cpu_vals}${m_psi_cpu}\n"
        psi_io_some_vals="${psi_io_some_vals}${m_psi_io_some}\n"
        psi_io_full_vals="${psi_io_full_vals}${m_psi_io_full}\n"
    done

    med_cpu=$(median "$(echo -e "$cpu_pcts" | head -n -1)")
    med_sys=$(median "$(echo -e "$sys_pcts" | head -n -1)")
    med_io_psi=$(median "$(echo -e "$io_psi_pcts" | head -n -1)")
    med_psi_cpu=$(median "$(echo -e "$psi_cpu_vals" | head -n -1)")
    med_psi_io_some=$(median "$(echo -e "$psi_io_some_vals" | head -n -1)")
    med_psi_io_full=$(median "$(echo -e "$psi_io_full_vals" | head -n -1)")

    echo "$n,0,0,0,0,0,$med_cpu,$med_sys,0.00,$med_io_psi,$med_psi_cpu,$med_psi_io_some,$med_psi_io_full" >> "$CSV_FILE"

    echo "  cpu=${med_cpu}% sys=${med_sys}%"
done

# Write experiment params
cat > "${RUN_DIR}/params.txt" <<EOF
experiment=ibench_${SOI}
soi=${SOI}
max_workers=${MAX_WORKERS}
duration=${DURATION}
samples=${SAMPLES}
step=${STEP}
warmup=${WARMUP}
num_cpus=${NUM_CPUS}
timestamp=${TIMESTAMP}
EOF

echo ""
echo "Done. Results in: $RUN_DIR"

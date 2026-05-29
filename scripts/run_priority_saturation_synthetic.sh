#!/usr/bin/env bash
# Synthetic "nice" (priority) experiment, mirroring the RocksDB nice sweep
# in completed_output/5_3/readrandomwriterandom_lowprio.
#
# Intended to run on the SECOND machine (Ryzen 9700X + SATA SSD) because the
# laptop's NVMe SSD is past its rated TBW and no longer gives trustworthy I/O.
#
# BEFORE RUNNING, on the desktop, edit compose.yaml:
#   - cpuset: pin to 4 cores (e.g. "0-3") so saturation dynamics stay
#     comparable to the laptop's 4-core setup. Do NOT let it use all 8/16.
#   - device_read_bps/device_write_bps/device_read_iops/device_write_iops:
#     change the `path:` from /dev/nvme0n1 to the desktop's SATA device
#     (find it with `lsblk` — likely /dev/sda). Keep rate 100mb / 10000.
#   - keep the 8 GB memory limit.
#
# Each SoI type is run twice with identical parameters:
#   [A] --nice 10  : SoI workers deprioritized (matches the RocksDB run).
#   [B] default    : SoI workers compete equally with the victim.
# Both CSVs carry soi_ops + victim columns, so the curves overlay.

set -euo pipefail
cd "$(dirname "$0")/.."

COMPOSE="docker compose run --build --remove-orphans saturator"
# Mirror the RocksDB run's measurement weight; cooldown lets I/O settle.
COMMON="--duration 30 --samples 7 --warmup 10 --cooldown 10 --sample-interval 200 --max-workers 8 --io-buffer-kb 64"

# Synthetic analog of the rrwr victim: a mixed CPU+I/O workload at saturation.
# NOTE: verify this count actually saturates on the desktop (4-core cpuset)
# before trusting absolute numbers — e.g. run
#   $COMPOSE find-mixed-saturation-proc 75 $COMMON
# once and adjust VICTIMS to the knee. The nice *effect* is a within-machine
# relative comparison, so it is robust to small mis-sizing.
VICTIMS=5
VICTIM_IO=75
NICE=10

echo ">>> Synthetic nice sweep: $VICTIMS victims @ ${VICTIM_IO}% IO, nice=$NICE <<<"

for SOI in cpu l1d l2 l3 membw memcap iobw iops; do
    echo ""
    echo "=============================================="
    echo "  SoI type: $SOI"
    echo "=============================================="

    echo "=== [A] Niced SoI (nice=$NICE) ==="
    $COMPOSE find-soi-sweep "$SOI" "$VICTIMS" "$VICTIM_IO" \
        --nice "$NICE" \
        $COMMON --output-dir priority_synth_${SOI}_niced

    echo "=== [B] Default priority SoI ==="
    $COMPOSE find-soi-sweep "$SOI" "$VICTIMS" "$VICTIM_IO" \
        $COMMON --output-dir priority_synth_${SOI}_default
done

echo ""
echo "=== Synthetic priority experiments complete ($VICTIMS victims @ ${VICTIM_IO}% IO, nice=$NICE) ==="

#!/usr/bin/env python3
"""
Generate graphs from saturator CSV output files.

Graphs are written into the same directory as each CSV.

Usage:
    python plot_results.py                    # Plot all CSVs under ./output
    python plot_results.py output/run_dir/    # Plot CSVs in a specific run
    python plot_results.py path/to/file.csv   # Plot a specific file
"""

import glob
import os
import sys
from pathlib import Path

from plotting.ext import (
    plot_ext_saturation,
    plot_ext_saturation_timeseries,
    plot_ext_soi,
    plot_ext_soi_comparison,
    plot_ext_timeseries,
    plot_per_sample_ext_saturation,
    plot_per_sample_ext_soi,
)
from plotting.intensity import plot_per_worker_intensity_sweep
from plotting.saturation import plot_per_worker_saturation, plot_saturation
from plotting.slack import plot_per_worker_proc_slack, plot_proc_slack, plot_slack
from plotting.soi import plot_per_worker_soi, plot_soi, plot_soi_comparison


def find_csvs(paths):
    """Find CSV files from given paths (files or directories)."""
    csv_files = []
    for p in paths:
        if os.path.isfile(p) and p.endswith('.csv'):
            csv_files.append(p)
        elif os.path.isdir(p):
            csv_files.extend(glob.glob(os.path.join(p, '**', '*.csv'), recursive=True))
    return csv_files


def dispatch(csv_path):
    """Route a CSV path to the correct plotter based on its filename."""
    name = Path(csv_path).stem

    if name.startswith('per_sample_ext_soi_'):
        plot_per_sample_ext_soi(csv_path)
    elif name == 'per_sample_ext_saturation':
        plot_per_sample_ext_saturation(csv_path)
    elif name.startswith('per_worker_'):
        if 'intensity_sweep' in name:
            plot_per_worker_intensity_sweep(csv_path)
        elif 'proc_slack' in name:
            plot_per_worker_proc_slack(csv_path)
        elif name.startswith('per_worker_soi_'):
            plot_per_worker_soi(csv_path)
        else:
            plot_per_worker_saturation(csv_path)
    elif name.startswith('timeseries_ext_soi_'):
        plot_ext_timeseries(csv_path)
    elif name == 'timeseries_ext_saturation':
        plot_ext_saturation_timeseries(csv_path)
    elif name.startswith('ext_soi_') and name.endswith('_throughput'):
        plot_ext_soi(csv_path)
    elif name == 'ext_saturation':
        plot_ext_saturation(csv_path)
    elif name.startswith('soi_') and name.endswith('_throughput'):
        plot_soi(csv_path)
    elif 'throughput_vs_threads' in name or 'throughput_vs_workers' in name:
        plot_saturation(csv_path)
    elif name.startswith('slack_'):
        plot_slack(csv_path)
    elif name.startswith('proc_slack_'):
        plot_proc_slack(csv_path)
    else:
        print(f"  Skipping unknown format: {name}")


def main():
    if len(sys.argv) > 1:
        csv_files = find_csvs(sys.argv[1:])
    else:
        csv_files = find_csvs(['output'])

    if not csv_files:
        print("No CSV files found. Run saturator experiments first.")
        print("Usage: python plot_results.py [csv_files_or_dirs...]")
        sys.exit(1)

    print(f"Plotting {len(csv_files)} file(s)...")

    for csv_path in sorted(csv_files):
        name = Path(csv_path).stem
        print(f"Processing: {name}")
        try:
            dispatch(csv_path)
        except Exception as e:
            print(f"  Error: {e}")

    # Group SoI throughput CSVs by parent directory for comparison plots
    soi_groups = {}
    ext_soi_groups = {}
    for csv_path in sorted(csv_files):
        name = Path(csv_path).stem
        if name.startswith('ext_soi_') and name.endswith('_throughput'):
            parent = str(Path(csv_path).parent.parent)
            ext_soi_groups.setdefault(parent, []).append(csv_path)
        elif name.startswith('soi_') and name.endswith('_throughput'):
            parent = str(Path(csv_path).parent.parent)
            soi_groups.setdefault(parent, []).append(csv_path)
    for group_csvs in soi_groups.values():
        try:
            plot_soi_comparison(group_csvs)
        except Exception as e:
            print(f"  Error in SoI comparison: {e}")
    for group_csvs in ext_soi_groups.values():
        try:
            plot_ext_soi_comparison(group_csvs)
        except Exception as e:
            print(f"  Error in ext SoI comparison: {e}")

    print("\nDone!")


if __name__ == '__main__':
    main()

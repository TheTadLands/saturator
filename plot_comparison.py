#!/usr/bin/env python3
"""
Compare saturator vs iBench experiments by plotting system metrics side-by-side.

Finds matching pairs of saturator and iBench runs in ./output and generates
overlay plots showing CPU utilization, PSI pressure, and (for saturator only)
throughput curves.

Usage:
    python plot_comparison.py                              # Auto-find pairs
    python plot_comparison.py <saturator_csv> <ibench_csv> # Explicit pair
"""

import sys
import os
import glob
import pandas as pd
import matplotlib.pyplot as plt
from matplotlib.ticker import MaxNLocator
from pathlib import Path

# Colorblind-friendly palette
C_BLUE = '#0072B2'
C_ORANGE = '#E69F00'
C_GREEN = '#009E73'
C_RED = '#D55E00'
C_CYAN = '#56B4E9'
C_PURPLE = '#CC79A7'


def format_number(x):
    if x >= 1_000_000:
        return f'{x/1_000_000:.1f}M'
    elif x >= 1_000:
        return f'{x/1_000:.1f}K'
    else:
        return f'{x:.0f}'


def plot_comparison(sat_csv, ibench_csv, output_dir=None):
    """Generate comparison plots from a saturator CSV and an iBench CSV."""
    sat_df = pd.read_csv(sat_csv)
    ib_df = pd.read_csv(ibench_csv)

    sat_name = Path(sat_csv).parent.name
    ib_name = Path(ibench_csv).parent.name

    if output_dir is None:
        output_dir = str(Path(sat_csv).parent.parent / f'comparison_{Path(sat_csv).parent.name}_vs_{Path(ibench_csv).parent.name}')
    os.makedirs(output_dir, exist_ok=True)

    # Align on worker count range
    max_workers = min(sat_df['workers'].max(), ib_df['workers'].max())
    sat_df = sat_df[sat_df['workers'] <= max_workers]
    ib_df = ib_df[ib_df['workers'] <= max_workers]

    has_cpu_pct = 'cpu_pct' in sat_df.columns and 'cpu_pct' in ib_df.columns

    print(f"Comparing: {sat_name} vs {ib_name}")
    print(f"  Worker range: 1-{max_workers}")
    print(f"  Output: {output_dir}/")

    # 1. CPU utilization overlay
    if has_cpu_pct:
        fig, ax = plt.subplots(figsize=(10, 5))
        ax.plot(sat_df['workers'], sat_df['cpu_pct'], '-o', color=C_BLUE,
                linewidth=2, markersize=6, label='Saturator CPU%')
        ax.plot(ib_df['workers'], ib_df['cpu_pct'], '-s', color=C_RED,
                linewidth=2, markersize=6, label='iBench CPU%')
        ax.set_xlabel('Worker Count')
        ax.set_ylabel('CPU Utilization (%)')
        ax.set_title('CPU Utilization: Saturator vs iBench')
        ax.set_ylim(0, 105)
        ax.set_xlim(left=1)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend()
        ax.grid(True, alpha=0.3)
        fig.tight_layout()
        fig.savefig(os.path.join(output_dir, 'cpu_utilization_comparison.png'), dpi=150)
        plt.close(fig)
        print(f"    cpu_utilization_comparison.png")

    # 2. PSI CPU pressure overlay
    if 'psi_cpu_some_us' in sat_df.columns and 'psi_cpu_some_us' in ib_df.columns:
        fig, ax = plt.subplots(figsize=(10, 5))
        ax.plot(sat_df['workers'], sat_df['psi_cpu_some_us'], '-o', color=C_BLUE,
                linewidth=2, markersize=6, label='Saturator')
        ax.plot(ib_df['workers'], ib_df['psi_cpu_some_us'], '-s', color=C_RED,
                linewidth=2, markersize=6, label='iBench')
        ax.set_xlabel('Worker Count')
        ax.set_ylabel('CPU PSI (some) cumulative \u00b5s')
        ax.set_title('CPU Pressure Stall: Saturator vs iBench')
        ax.set_xlim(left=1)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend()
        ax.grid(True, alpha=0.3)
        ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
        fig.tight_layout()
        fig.savefig(os.path.join(output_dir, 'psi_cpu_comparison.png'), dpi=150)
        plt.close(fig)
        print(f"    psi_cpu_comparison.png")

    # 3. Combined: saturator throughput + CPU% for both on dual axes
    if has_cpu_pct:
        has_split = 'cpu_ops_sec' in sat_df.columns and 'io_ops_sec' in sat_df.columns
        if has_split:
            tp_col = 'cpu_ops_sec' if sat_df['cpu_ops_sec'].max() >= sat_df['io_ops_sec'].max() else 'io_ops_sec'
            tp_label = 'CPU ops/s' if tp_col == 'cpu_ops_sec' else 'IO ops/s'
        else:
            tp_col = 'throughput_ops_sec'
            tp_label = 'Throughput'

        fig, ax1 = plt.subplots(figsize=(10, 5))

        # Saturator throughput on left axis
        line_tp = ax1.plot(sat_df['workers'], sat_df[tp_col], '-o', color=C_BLUE,
                           linewidth=2, markersize=6, label=f'Saturator {tp_label}')
        ax1.set_xlabel('Worker Count')
        ax1.set_ylabel(f'{tp_label} (ops/sec)', color=C_BLUE)
        ax1.tick_params(axis='y', labelcolor=C_BLUE)
        ax1.set_ylim(bottom=0)
        ax1.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

        # CPU% for both on right axis
        ax2 = ax1.twinx()
        line_sat_cpu = ax2.plot(sat_df['workers'], sat_df['cpu_pct'], '--o', color=C_ORANGE,
                                linewidth=1.5, markersize=4, alpha=0.8, label='Saturator CPU%')
        line_ib_cpu = ax2.plot(ib_df['workers'], ib_df['cpu_pct'], '--s', color=C_RED,
                               linewidth=1.5, markersize=4, alpha=0.8, label='iBench CPU%')
        ax2.set_ylabel('CPU Utilization (%)')
        ax2.set_ylim(0, 105)

        lines = line_tp + line_sat_cpu + line_ib_cpu
        ax1.legend(lines, [l.get_label() for l in lines], loc='best')
        ax1.set_title('Saturator Throughput + CPU Comparison')
        ax1.set_xlim(left=1)
        ax1.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax1.grid(True, alpha=0.3)
        fig.tight_layout()
        fig.savefig(os.path.join(output_dir, 'throughput_vs_cpu_comparison.png'), dpi=150)
        plt.close(fig)
        print(f"    throughput_vs_cpu_comparison.png")

    # 4. System % (kernel time) comparison
    if 'system_pct' in sat_df.columns and 'system_pct' in ib_df.columns:
        fig, ax = plt.subplots(figsize=(10, 5))
        ax.plot(sat_df['workers'], sat_df['system_pct'], '-o', color=C_BLUE,
                linewidth=2, markersize=6, label='Saturator system%')
        ax.plot(ib_df['workers'], ib_df['system_pct'], '-s', color=C_RED,
                linewidth=2, markersize=6, label='iBench system%')
        ax.set_xlabel('Worker Count')
        ax.set_ylabel('System (kernel) CPU %')
        ax.set_title('Kernel CPU Time: Saturator vs iBench')
        ax.set_ylim(bottom=0)
        ax.set_xlim(left=1)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend()
        ax.grid(True, alpha=0.3)
        fig.tight_layout()
        fig.savefig(os.path.join(output_dir, 'system_pct_comparison.png'), dpi=150)
        plt.close(fig)
        print(f"    system_pct_comparison.png")

    print("Done.")


def find_pairs(base_dir='output'):
    """Auto-discover saturator + iBench CSV pairs."""
    sat_csvs = glob.glob(os.path.join(base_dir, '**/*throughput_vs_workers*.csv'), recursive=True)
    ib_csvs = glob.glob(os.path.join(base_dir, '**/*ibench_*throughput_vs_workers*.csv'), recursive=True)

    # Filter: saturator CSVs are those NOT starting with 'ibench_'
    sat_csvs = [c for c in sat_csvs if 'ibench_' not in Path(c).stem]

    return sat_csvs, ib_csvs


def main():
    if len(sys.argv) == 3:
        plot_comparison(sys.argv[1], sys.argv[2])
        return

    sat_csvs, ib_csvs = find_pairs()

    if not sat_csvs:
        print("No saturator CSVs found. Run a saturator experiment first.")
        sys.exit(1)
    if not ib_csvs:
        print("No iBench CSVs found. Run an iBench experiment first.")
        sys.exit(1)

    print(f"Found {len(sat_csvs)} saturator CSV(s) and {len(ib_csvs)} iBench CSV(s)")

    # Compare each iBench CSV against the most recent saturator CSV of matching type
    for ib_csv in sorted(ib_csvs):
        ib_name = Path(ib_csv).stem
        # Determine experiment type from iBench name (cpu, io, mixed)
        is_cpu = 'cpu' in ib_name.lower()

        # Find best matching saturator CSV
        candidates = []
        for s in sat_csvs:
            s_name = Path(s).stem.lower()
            s_is_cpu = 'cpu' in s_name
            if s_is_cpu == is_cpu:
                candidates.append(s)

        if not candidates:
            candidates = sat_csvs  # Fall back to any

        # Use most recent (by modification time)
        best = max(candidates, key=os.path.getmtime)
        print(f"\nPairing: {Path(best).parent.name} <-> {Path(ib_csv).parent.name}")
        plot_comparison(best, ib_csv)


if __name__ == '__main__':
    main()

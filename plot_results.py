#!/usr/bin/env python3
"""
Generate graphs from saturator CSV output files.

Graphs are written into the same directory as each CSV.

Usage:
    python plot_results.py                    # Plot all CSVs under ./output
    python plot_results.py output/run_dir/    # Plot CSVs in a specific run
    python plot_results.py path/to/file.csv   # Plot a specific file
"""

import sys
import os
import glob
import pandas as pd
import matplotlib.pyplot as plt
from pathlib import Path

# Colorblind-friendly palette (Wong 2011)
C_BLUE = '#0072B2'
C_ORANGE = '#E69F00'
C_GREEN = '#009E73'
C_RED = '#D55E00'
C_PURPLE = '#CC79A7'
C_CYAN = '#56B4E9'
C_YELLOW = '#F0E442'
C_BLACK = '#000000'


def format_number(x):
    """Format large numbers with K/M suffixes."""
    if x >= 1_000_000:
        return f'{x/1_000_000:.1f}M'
    elif x >= 1_000:
        return f'{x/1_000:.0f}K'
    else:
        return f'{x:.0f}'


def save_fig(fig, path):
    fig.tight_layout()
    fig.savefig(path, dpi=150)
    plt.close(fig)
    print(f"    {Path(path).name}")


def detect_experiment(name):
    """Return (label, mode, x_label) from CSV stem name."""
    is_cpu = 'cpu' in name.lower()
    is_proc = 'proc' in name.lower() or 'worker' in name.lower()
    label = 'CPU' if is_cpu else 'I/O'
    mode = 'Process' if is_proc else 'Thread'
    x_label = 'Worker Count' if is_proc else 'Thread Count'
    return label, mode, x_label


def plot_saturation(csv_path: str):
    """Generate graphs alongside a saturation CSV."""
    df = pd.read_csv(csv_path)
    name = Path(csv_path).stem
    folder = str(Path(csv_path).parent)
    label, mode, x_label = detect_experiment(name)
    x = df['threads']

    # Support both old (throughput_ops_sec) and new (cpu_ops_sec/io_ops_sec) CSV formats
    has_split = 'cpu_ops_sec' in df.columns and 'io_ops_sec' in df.columns
    if has_split:
        df['total_ops'] = df.get('total_ops_sec', df['cpu_ops_sec'] + df['io_ops_sec'])
    else:
        df['total_ops'] = df['throughput_ops_sec']

    has_cpu_stddev = 'cpu_ops_stddev' in df.columns
    has_io_stddev = 'io_ops_stddev' in df.columns

    print(f"  -> {folder}/")

    # 1. Throughput breakdown (CPU + IO + total)
    fig, ax = plt.subplots(figsize=(8, 5))
    if has_split:
        ax.errorbar(x, df['cpu_ops_sec'],
                    yerr=df['cpu_ops_stddev'] if has_cpu_stddev else None,
                    fmt='-o', color=C_BLUE, linewidth=2, markersize=6,
                    capsize=3, capthick=1, label='CPU ops/s')
        ax.errorbar(x, df['io_ops_sec'],
                    yerr=df['io_ops_stddev'] if has_io_stddev else None,
                    fmt='-s', color=C_GREEN, linewidth=2, markersize=6,
                    capsize=3, capthick=1, label='IO ops/s')
        ax.plot(x, df['total_ops'], '-^', color=C_BLACK, linewidth=1.5,
                markersize=5, alpha=0.7, label='Total')
    else:
        yerr = df.get('throughput_stddev')
        ax.errorbar(x, df['total_ops'], yerr=yerr, fmt='-o', color=C_BLUE,
                    linewidth=2, markersize=6, capsize=3, capthick=1)
    peak_idx = df['total_ops'].idxmax()
    ax.axvline(x=df.loc[peak_idx, 'threads'], color=C_RED, linestyle='--', alpha=0.7,
               label=f'Peak: {df.loc[peak_idx, "threads"]} {mode.lower()}s')
    ax.scatter([df.loc[peak_idx, 'threads']], [df.loc[peak_idx, 'total_ops']],
               color=C_RED, s=100, zorder=5)
    ax.set_xlabel(x_label)
    ax.set_ylabel('Throughput (ops/sec)')
    ax.set_title(f'{label} Saturation ({mode}s) — Throughput')
    ax.legend()
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'throughput_total.png'))

    # 2. Per-worker throughput
    fig, ax = plt.subplots(figsize=(8, 5))
    ax.plot(x, df['throughput_per_thread'], '-s', color=C_BLUE,
            linewidth=2, markersize=6)
    first_tp = df.loc[0, 'throughput_per_thread']
    last_tp = df.loc[len(df)-1, 'throughput_per_thread']
    drop = (1 - last_tp / first_tp) * 100
    ax.annotate(f'{drop:.0f}% efficiency loss',
                xy=(x.iloc[-1], last_tp),
                xytext=(x.iloc[-1] * 0.7, (first_tp + last_tp) / 2),
                arrowprops=dict(arrowstyle='->', color='gray'),
                fontsize=10, color='gray')
    ax.set_xlabel(x_label)
    ax.set_ylabel(f'Throughput per {mode} (ops/sec)')
    ax.set_title(f'{label} Saturation ({mode}s) — Per {mode} Efficiency')
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'throughput_per_worker.png'))

    # 3. CPU & IO utilization vs workers
    if 'cpu_pct' in df.columns:
        fig, ax = plt.subplots(figsize=(8, 5))
        ax.plot(x, df['cpu_pct'], '-o', color=C_ORANGE, linewidth=2, markersize=6, label='CPU %')
        if 'io_util_pct' in df.columns:
            ax.plot(x, df['io_util_pct'], '-s', color=C_GREEN, linewidth=2, markersize=6, label='IO %')
        ax.set_xlabel(x_label)
        ax.set_ylabel('Utilization (%)')
        ax.set_title(f'{label} Saturation ({mode}s) — Resource Utilization')
        ax.set_ylim(0, 100)
        ax.legend()
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'utilization.png'))

    # 4. Combined: throughput + CPU% on dual axes
    if 'cpu_pct' in df.columns:
        fig, ax1 = plt.subplots(figsize=(8, 5))
        ln1 = ax1.plot(x, df['total_ops'], '-o', color=C_BLUE, linewidth=2,
                       markersize=6, label='Throughput')
        ax1.set_xlabel(x_label)
        ax1.set_ylabel('Throughput (ops/sec)', color=C_BLUE)
        ax1.tick_params(axis='y', labelcolor=C_BLUE)
        ax1.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

        ax2 = ax1.twinx()
        ln2 = ax2.plot(x, df['cpu_pct'], '-s', color=C_ORANGE, linewidth=2,
                       markersize=5, label='CPU %')
        ax2.set_ylabel('CPU Utilization (%)', color=C_ORANGE)
        ax2.tick_params(axis='y', labelcolor=C_ORANGE)
        ax2.set_ylim(bottom=0)

        lines = ln1 + ln2
        labels = [l.get_label() for l in lines]
        ax1.legend(lines, labels, loc='best')
        ax1.set_title(f'{label} Saturation ({mode}s) — Throughput vs CPU')
        ax1.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'throughput_vs_cpu.png'))

    # 5. Normalized: throughput % of peak vs utilization %
    if 'cpu_pct' in df.columns:
        fig, ax = plt.subplots(figsize=(8, 5))

        if has_split:
            cpu_peak = df['cpu_ops_sec'].max()
            io_peak = df['io_ops_sec'].max()
            if cpu_peak > 0:
                ax.plot(x, df['cpu_ops_sec'] / cpu_peak * 100, '-o', color=C_BLUE,
                        linewidth=2, markersize=5, label='CPU ops (% of peak)')
            if io_peak > 0:
                ax.plot(x, df['io_ops_sec'] / io_peak * 100, '-s', color=C_CYAN,
                        linewidth=2, markersize=5, label='IO ops (% of peak)')
        else:
            total_peak = df['total_ops'].max()
            if total_peak > 0:
                ax.plot(x, df['total_ops'] / total_peak * 100, '-o', color=C_BLUE,
                        linewidth=2, markersize=5, label='Throughput (% of peak)')

        ax.plot(x, df['cpu_pct'], '--', color=C_ORANGE, linewidth=2,
                markersize=4, alpha=0.8, label='CPU utilization %')
        if 'io_util_pct' in df.columns:
            ax.plot(x, df['io_util_pct'], '--', color=C_GREEN, linewidth=2,
                    markersize=4, alpha=0.8, label='IO utilization %')

        ax.set_xlabel(x_label)
        ax.set_ylabel('% of Peak / Utilization %')
        ax.set_title(f'{label} Saturation ({mode}s) — Throughput vs Utilization')
        ax.set_ylim(0, 105)
        ax.legend(loc='best', fontsize=9)
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'throughput_vs_utilization.png'))


def plot_slack(csv_path: str):
    """Generate graphs alongside a slack CSV."""
    df = pd.read_csv(csv_path)
    name = Path(csv_path).stem
    folder = str(Path(csv_path).parent)

    is_cpu_baseline = 'cpu_adding' in name or name.startswith('cpu_slack')
    baseline_type = 'CPU' if is_cpu_baseline else 'I/O'
    x = df['extra_threads']

    has_cpu_stddev = 'cpu_ops_stddev' in df.columns
    has_io_stddev = 'io_ops_stddev' in df.columns

    print(f"  -> {folder}/")

    # 1. Throughput breakdown (dual y-axes)
    fig, ax1 = plt.subplots(figsize=(8, 5))
    ln1 = ax1.errorbar(x, df['cpu_ops'],
                       yerr=df['cpu_ops_stddev'] if has_cpu_stddev else None,
                       fmt='o-', color=C_BLUE, label='CPU ops/s',
                       linewidth=2, markersize=5, capsize=3, capthick=1)
    ax1.set_xlabel('Extra Threads')
    ax1.set_ylabel('CPU ops/sec', color=C_BLUE)
    ax1.tick_params(axis='y', labelcolor=C_BLUE)
    ax1.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

    ax1_io = ax1.twinx()
    ln2 = ax1_io.errorbar(x, df['io_ops'],
                          yerr=df['io_ops_stddev'] if has_io_stddev else None,
                          fmt='s-', color=C_GREEN, label='I/O ops/s',
                          linewidth=2, markersize=5, capsize=3, capthick=1)
    ax1_io.set_ylabel('I/O ops/sec', color=C_GREEN)
    ax1_io.tick_params(axis='y', labelcolor=C_GREEN)
    ax1_io.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

    ax1.legend(handles=[ln1, ln2], loc='best')
    ax1.set_title(f'{name.replace("_", " ").title()} — Throughput')
    ax1.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'throughput.png'))

    # 2. Baseline degradation
    fig, ax = plt.subplots(figsize=(8, 5))
    ax.plot(x, df['baseline_change_pct'], '-o', color=C_RED, linewidth=2, markersize=5)
    ax.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
    ax.axhline(y=-20, color=C_ORANGE, linestyle='--', alpha=0.7, label='-20% threshold')
    ax.fill_between(x, df['baseline_change_pct'], 0,
                    where=(df['baseline_change_pct'] >= 0), alpha=0.2, color=C_GREEN)
    ax.fill_between(x, df['baseline_change_pct'], 0,
                    where=(df['baseline_change_pct'] < 0), alpha=0.2, color=C_RED)
    ax.set_xlabel('Extra Threads')
    ax.set_ylabel(f'{baseline_type} Baseline Change (%)')
    ax.set_title(f'{baseline_type} Baseline Degradation')
    ax.legend()
    ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'baseline_change.png'))

    # 3. CPU & IO utilization vs extra threads
    if 'cpu_pct' in df.columns:
        fig, ax = plt.subplots(figsize=(8, 5))
        ax.plot(x, df['cpu_pct'], '-o', color=C_ORANGE, linewidth=2, markersize=6, label='CPU %')
        if 'io_util_pct' in df.columns:
            ax.plot(x, df['io_util_pct'], '-s', color=C_GREEN, linewidth=2, markersize=6, label='IO %')
        ax.set_xlabel('Extra Threads')
        ax.set_ylabel('Utilization (%)')
        ax.set_title(f'{name.replace("_", " ").title()} — Resource Utilization')
        ax.set_ylim(0, 100)
        ax.legend()
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'utilization.png'))


def find_csvs(paths):
    """Find CSV files from given paths (files or directories)."""
    csv_files = []
    for p in paths:
        if os.path.isfile(p) and p.endswith('.csv'):
            csv_files.append(p)
        elif os.path.isdir(p):
            csv_files.extend(glob.glob(os.path.join(p, '**', '*.csv'), recursive=True))
    return csv_files


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
            if 'throughput_vs_threads' in name or 'throughput_vs_workers' in name:
                plot_saturation(csv_path)
            elif name.startswith('slack'):
                plot_slack(csv_path)
            else:
                print(f"  Skipping unknown format: {name}")
        except Exception as e:
            print(f"  Error: {e}")

    print("\nDone!")


if __name__ == '__main__':
    main()

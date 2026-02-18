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
    if not has_split:
        df['total_ops'] = df['throughput_ops_sec']

    has_cpu_stddev = 'cpu_ops_stddev' in df.columns
    has_io_stddev = 'io_ops_stddev' in df.columns

    # Primary metric: whichever of CPU/IO has the higher peak (handles pure and mixed experiments)
    if has_split:
        primary_col = 'cpu_ops_sec' if df['cpu_ops_sec'].max() >= df['io_ops_sec'].max() else 'io_ops_sec'
        primary_label = 'CPU ops/s' if primary_col == 'cpu_ops_sec' else 'IO ops/s'
    else:
        primary_col = 'total_ops'
        primary_label = 'Throughput'

    print(f"  -> {folder}/")

    # 1. Throughput breakdown (CPU + IO)
    peak_idx = df[primary_col].idxmax()
    if has_split:
        fig, ax1 = plt.subplots(figsize=(8, 5))
        ln1 = ax1.errorbar(x, df['cpu_ops_sec'],
                           yerr=df['cpu_ops_stddev'] if has_cpu_stddev else None,
                           fmt='-o', color=C_BLUE, linewidth=2, markersize=6,
                           capsize=3, capthick=1, label='CPU ops/s')
        ax1.set_xlabel(x_label)
        ax1.set_ylabel('CPU ops/sec', color=C_BLUE)
        ax1.tick_params(axis='y', labelcolor=C_BLUE)
        ax1.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

        ax2 = ax1.twinx()
        ln2 = ax2.errorbar(x, df['io_ops_sec'],
                           yerr=df['io_ops_stddev'] if has_io_stddev else None,
                           fmt='-s', color=C_GREEN, linewidth=2, markersize=6,
                           capsize=3, capthick=1, label='IO ops/s')
        ax2.set_ylabel('IO ops/sec', color=C_GREEN)
        ax2.tick_params(axis='y', labelcolor=C_GREEN)
        ax2.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

        peak_ax = ax1 if primary_col == 'cpu_ops_sec' else ax2
        peak_ax.axvline(x=df.loc[peak_idx, 'threads'], color=C_RED, linestyle='--', alpha=0.7,
                        label=f'Peak: {df.loc[peak_idx, "threads"]} {mode.lower()}s')
        peak_ax.scatter([df.loc[peak_idx, 'threads']], [df.loc[peak_idx, primary_col]],
                        color=C_RED, s=100, zorder=5)
        ax1.legend(handles=[ln1, ln2], loc='best')
        ax1.set_title(f'{label} Saturation ({mode}s) — Throughput')
        ax1.grid(True, alpha=0.3)
    else:
        fig, ax = plt.subplots(figsize=(8, 5))
        yerr = df.get('throughput_stddev')
        ax.errorbar(x, df['total_ops'], yerr=yerr, fmt='-o', color=C_BLUE,
                    linewidth=2, markersize=6, capsize=3, capthick=1)
        ax.axvline(x=df.loc[peak_idx, 'threads'], color=C_RED, linestyle='--', alpha=0.7,
                   label=f'Peak: {df.loc[peak_idx, "threads"]} {mode.lower()}s')
        ax.scatter([df.loc[peak_idx, 'threads']], [df.loc[peak_idx, primary_col]],
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

    # 4. Combined: primary throughput metric + CPU% on dual axes
    if 'cpu_pct' in df.columns:
        fig, ax1 = plt.subplots(figsize=(8, 5))
        ln1 = ax1.plot(x, df[primary_col], '-o', color=C_BLUE, linewidth=2,
                       markersize=6, label=primary_label)
        ax1.set_xlabel(x_label)
        ax1.set_ylabel(f'{primary_label} (ops/sec)', color=C_BLUE)
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

    # 4. Normalized throughput % of peak vs utilization %
    if 'cpu_pct' in df.columns:
        fig, ax = plt.subplots(figsize=(8, 5))
        cpu_peak = df['cpu_ops'].max()
        io_peak = df['io_ops'].max()
        if cpu_peak > 0:
            ax.plot(x, df['cpu_ops'] / cpu_peak * 100, '-o', color=C_BLUE,
                    linewidth=2, markersize=5, label='CPU ops (% of peak)')
        if io_peak > 0:
            ax.plot(x, df['io_ops'] / io_peak * 100, '-s', color=C_CYAN,
                    linewidth=2, markersize=5, label='IO ops (% of peak)')
        ax.plot(x, df['cpu_pct'], '--', color=C_ORANGE, linewidth=2,
                markersize=4, alpha=0.8, label='CPU utilization %')
        if 'io_util_pct' in df.columns:
            ax.plot(x, df['io_util_pct'], '--', color=C_GREEN, linewidth=2,
                    markersize=4, alpha=0.8, label='IO utilization %')
        ax.set_xlabel('Extra Threads')
        ax.set_ylabel('% of Peak / Utilization %')
        ax.set_title(f'{baseline_type} Slack — Throughput vs Utilization')
        ax.set_ylim(0, 105)
        ax.legend(loc='best', fontsize=9)
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'throughput_vs_utilization.png'))


def plot_per_worker_saturation(csv_path: str):
    """Box plot + fairness line for per-worker saturation CSVs."""
    df = pd.read_csv(csv_path)
    name = Path(csv_path).stem
    folder = str(Path(csv_path).parent)
    label, mode, x_label = detect_experiment(name)

    worker_counts = sorted(df['workers'].unique())
    box_width = (worker_counts[1] - worker_counts[0]) * 0.6 if len(worker_counts) > 1 else 0.6

    print(f"  -> {folder}/")

    # 1. Box plot of per-worker throughput distribution
    fig, ax = plt.subplots(figsize=(max(8, len(worker_counts) * 0.4), 5))
    data = [df[df['workers'] == w]['total_ops_sec'].values for w in worker_counts]
    ax.boxplot(data, positions=worker_counts, widths=box_width, patch_artist=True,
               boxprops=dict(facecolor=C_CYAN, alpha=0.6),
               medianprops=dict(color=C_BLUE, linewidth=2))
    ax.set_xlabel(x_label)
    ax.set_ylabel('Per-Worker Throughput (ops/sec)')
    ax.set_title(f'{label} Saturation ({mode}s) — Per-Worker Distribution')
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'per_worker_distribution.png'))

    # 2. Total throughput + coefficient of variation (fairness)
    totals = [df[df['workers'] == w]['total_ops_sec'].sum() for w in worker_counts]
    cvs = []
    for w in worker_counts:
        vals = df[df['workers'] == w]['total_ops_sec'].values
        mean = vals.mean()
        cvs.append(vals.std() / mean * 100 if mean > 0 else 0)

    fig, ax1 = plt.subplots(figsize=(8, 5))
    ln1 = ax1.plot(worker_counts, totals, '-o', color=C_BLUE, linewidth=2, markersize=6, label='Total throughput')
    ax1.set_xlabel(x_label)
    ax1.set_ylabel('Total Throughput (ops/sec)', color=C_BLUE)
    ax1.tick_params(axis='y', labelcolor=C_BLUE)
    ax1.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

    ax2 = ax1.twinx()
    ln2 = ax2.plot(worker_counts, cvs, '-s', color=C_RED, linewidth=2, markersize=5, label='Fairness CV%')
    ax2.set_ylabel('Coefficient of Variation (%)', color=C_RED)
    ax2.tick_params(axis='y', labelcolor=C_RED)
    ax2.set_ylim(bottom=0)

    lines = ln1 + ln2
    ax1.legend(lines, [l.get_label() for l in lines], loc='best')
    ax1.set_title(f'{label} Saturation ({mode}s) — Throughput vs Fairness')
    ax1.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'per_worker_fairness.png'))


def plot_proc_slack(csv_path: str):
    """Generate graphs for proc slack CSVs."""
    df = pd.read_csv(csv_path)
    name = Path(csv_path).stem
    folder = str(Path(csv_path).parent)

    baseline_workers = int(df['baseline_workers'].iloc[0])
    is_io_baseline = 'io' in name.split('proc_slack_')[1].split('proc_adding')[0]
    baseline_label = 'I/O' if is_io_baseline else 'CPU'
    tracked_col = 'baseline_io_ops' if is_io_baseline else 'baseline_cpu_ops'
    x = df['extra_workers']

    print(f"  -> {folder}/")

    # 1. Baseline throughput degradation vs extra workers (CPU left axis, IO right axis)
    fig, ax1 = plt.subplots(figsize=(8, 5))
    ln1 = ax1.errorbar(x, df['baseline_cpu_ops'],
                       yerr=df['baseline_cpu_stddev'],
                       fmt='o-', color=C_BLUE, linewidth=2, markersize=5,
                       capsize=3, capthick=1, label=f'Baseline CPU ops/s ({baseline_workers} procs)')
    ln3 = ax1.plot(x, df['extra_cpu_ops'], '--^', color=C_CYAN, linewidth=1.5,
                   markersize=4, alpha=0.8, label='Extra CPU ops/s')
    ax1.set_xlabel('Extra Worker Processes')
    ax1.set_ylabel('CPU ops/sec', color=C_BLUE)
    ax1.tick_params(axis='y', labelcolor=C_BLUE)
    ax1.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

    ax2 = ax1.twinx()
    ln2 = ax2.errorbar(x, df['baseline_io_ops'],
                       yerr=df['baseline_io_stddev'],
                       fmt='s-', color=C_GREEN, linewidth=2, markersize=5,
                       capsize=3, capthick=1, label=f'Baseline IO ops/s ({baseline_workers} procs)')
    ln4 = ax2.plot(x, df['extra_io_ops'], '--v', color=C_ORANGE, linewidth=1.5,
                   markersize=4, alpha=0.8, label='Extra IO ops/s')
    ax2.set_ylabel('IO ops/sec', color=C_GREEN)
    ax2.tick_params(axis='y', labelcolor=C_GREEN)
    ax2.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    ax2.set_ylim(bottom=0)

    lines = [ln1] + ln3 + [ln2] + ln4
    ax1.legend(lines, [l.get_label() for l in lines], loc='best', fontsize=8)
    ax1.set_title(f'Proc Slack ({baseline_label} baseline, {baseline_workers} procs) — Throughput')
    ax1.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'throughput.png'))

    # 2. Baseline change % with zero line
    fig, ax = plt.subplots(figsize=(8, 5))
    ax.plot(x, df['baseline_change_pct'], '-o', color=C_RED, linewidth=2, markersize=5)
    ax.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
    ax.axhline(y=-20, color=C_ORANGE, linestyle='--', alpha=0.7, label='-20% threshold')
    ax.fill_between(x, df['baseline_change_pct'], 0,
                    where=(df['baseline_change_pct'] >= 0), alpha=0.2, color=C_GREEN)
    ax.fill_between(x, df['baseline_change_pct'], 0,
                    where=(df['baseline_change_pct'] < 0), alpha=0.2, color=C_RED)
    ax.set_xlabel('Extra Worker Processes')
    ax.set_ylabel(f'{baseline_label} Baseline Change (%)')
    ax.set_title(f'Proc Slack — Baseline Degradation ({baseline_label} baseline, {baseline_workers} procs)')
    ax.legend()
    ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'baseline_change.png'))

    # 3. Utilization
    if 'cpu_pct' in df.columns:
        fig, ax = plt.subplots(figsize=(8, 5))
        ax.plot(x, df['cpu_pct'], '-o', color=C_ORANGE, linewidth=2, markersize=6, label='CPU %')
        if 'io_util_pct' in df.columns:
            ax.plot(x, df['io_util_pct'], '-s', color=C_GREEN, linewidth=2, markersize=6, label='IO %')
        ax.set_xlabel('Extra Worker Processes')
        ax.set_ylabel('Utilization (%)')
        ax.set_title(f'Proc Slack — Resource Utilization')
        ax.set_ylim(0, 100)
        ax.legend()
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'utilization.png'))

    # 4. Normalized throughput % of peak vs utilization %
    if 'cpu_pct' in df.columns:
        fig, ax = plt.subplots(figsize=(8, 5))
        b_cpu_peak = df['baseline_cpu_ops'].max()
        b_io_peak = df['baseline_io_ops'].max()
        e_cpu_peak = df['extra_cpu_ops'].max()
        e_io_peak = df['extra_io_ops'].max()
        if b_cpu_peak > 0:
            ax.plot(x, df['baseline_cpu_ops'] / b_cpu_peak * 100, '-o', color=C_BLUE,
                    linewidth=2, markersize=5, label='Baseline CPU ops (% of peak)')
        if b_io_peak > 0:
            ax.plot(x, df['baseline_io_ops'] / b_io_peak * 100, '-s', color=C_CYAN,
                    linewidth=2, markersize=5, label='Baseline IO ops (% of peak)')
        if e_cpu_peak > 0:
            ax.plot(x, df['extra_cpu_ops'] / e_cpu_peak * 100, '--^', color=C_BLUE,
                    linewidth=1.5, markersize=4, alpha=0.8, label='Extra CPU ops (% of peak)')
        if e_io_peak > 0:
            ax.plot(x, df['extra_io_ops'] / e_io_peak * 100, '--v', color=C_CYAN,
                    linewidth=1.5, markersize=4, alpha=0.8, label='Extra IO ops (% of peak)')
        ax.plot(x, df['cpu_pct'], '--', color=C_ORANGE, linewidth=2,
                markersize=4, alpha=0.8, label='CPU utilization %')
        if 'io_util_pct' in df.columns:
            ax.plot(x, df['io_util_pct'], '--', color=C_GREEN, linewidth=2,
                    markersize=4, alpha=0.8, label='IO utilization %')
        ax.set_xlabel('Extra Worker Processes')
        ax.set_ylabel('% of Peak / Utilization %')
        ax.set_title(f'Proc Slack ({baseline_label} baseline, {baseline_workers} procs) — Throughput vs Utilization')
        ax.set_ylim(0, 105)
        ax.legend(loc='best', fontsize=9)
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'throughput_vs_utilization.png'))


def plot_per_worker_proc_slack(csv_path: str):
    """Box plot + fairness line for proc slack per-worker CSVs."""
    df = pd.read_csv(csv_path)
    folder = str(Path(csv_path).parent)

    baseline_workers = int(df['baseline_workers'].iloc[0])
    extra_counts = sorted(df['extra_workers'].unique())
    box_width = (extra_counts[1] - extra_counts[0]) * 0.6 if len(extra_counts) > 1 else 0.6

    print(f"  -> {folder}/")

    base_df  = df[df['worker_id'] < baseline_workers]
    extra_df = df[df['worker_id'] >= baseline_workers]

    # 1. Box plot separating baseline vs extra workers
    fig, ax = plt.subplots(figsize=(max(8, len(extra_counts) * 0.5), 5))
    base_data  = [base_df[base_df['extra_workers'] == e]['total_ops_sec'].values for e in extra_counts]
    extra_data = [extra_df[extra_df['extra_workers'] == e]['total_ops_sec'].values for e in extra_counts]

    offsets = [-box_width * 0.3, box_width * 0.3]
    bp1 = ax.boxplot(base_data, positions=[e + offsets[0] for e in extra_counts],
                     widths=box_width * 0.55, patch_artist=True,
                     boxprops=dict(facecolor=C_CYAN, alpha=0.7),
                     medianprops=dict(color=C_BLUE, linewidth=2), manage_ticks=False)
    bp2 = ax.boxplot(extra_data, positions=[e + offsets[1] for e in extra_counts],
                     widths=box_width * 0.55, patch_artist=True,
                     boxprops=dict(facecolor=C_ORANGE, alpha=0.7),
                     medianprops=dict(color=C_RED, linewidth=2), manage_ticks=False)

    ax.set_xticks(extra_counts)
    ax.set_xticklabels(extra_counts)
    ax.set_xlabel('Extra Worker Processes')
    ax.set_ylabel('Per-Worker Throughput (ops/sec)')
    ax.set_title(f'Proc Slack — Per-Worker Distribution (blue=baseline, orange=extra)')
    ax.legend([bp1['boxes'][0], bp2['boxes'][0]], [f'Baseline ({baseline_workers})', 'Extra'], loc='best')
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'per_worker_distribution.png'))

    # 2. Baseline fairness: total baseline throughput + CV%
    totals = [base_df[base_df['extra_workers'] == e]['total_ops_sec'].sum() for e in extra_counts]
    cvs = []
    for e in extra_counts:
        vals = base_df[base_df['extra_workers'] == e]['total_ops_sec'].values
        mean = vals.mean()
        cvs.append(vals.std() / mean * 100 if mean > 0 else 0)

    fig, ax1 = plt.subplots(figsize=(8, 5))
    ln1 = ax1.plot(extra_counts, totals, '-o', color=C_BLUE, linewidth=2, markersize=6, label='Baseline total throughput')
    ax1.set_xlabel('Extra Worker Processes')
    ax1.set_ylabel('Baseline Total Throughput (ops/sec)', color=C_BLUE)
    ax1.tick_params(axis='y', labelcolor=C_BLUE)
    ax1.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

    ax2 = ax1.twinx()
    ln2 = ax2.plot(extra_counts, cvs, '-s', color=C_RED, linewidth=2, markersize=5, label='Baseline fairness CV%')
    ax2.set_ylabel('Coefficient of Variation (%)', color=C_RED)
    ax2.tick_params(axis='y', labelcolor=C_RED)
    ax2.set_ylim(bottom=0)

    lines = ln1 + ln2
    ax1.legend(lines, [l.get_label() for l in lines], loc='best')
    ax1.set_title(f'Proc Slack — Baseline Throughput vs Fairness')
    ax1.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'per_worker_fairness.png'))


def plot_per_worker_intensity_sweep(csv_path: str):
    """Box plot of base workers + probe worker line for intensity sweep per-worker CSVs."""
    df = pd.read_csv(csv_path)
    folder = str(Path(csv_path).parent)

    intensities = sorted(df['probe_intensity'].unique())
    total_workers = int(df['workers'].iloc[0])
    probe_id = total_workers - 1

    base_df = df[df['worker_id'] != probe_id]
    probe_df = df[df['worker_id'] == probe_id]

    print(f"  -> {folder}/")

    fig, ax = plt.subplots(figsize=(10, 5))

    data = [base_df[base_df['probe_intensity'] == i]['total_ops_sec'].values for i in intensities]
    ax.boxplot(data, positions=intensities, widths=0.03, patch_artist=True,
               boxprops=dict(facecolor=C_CYAN, alpha=0.6),
               medianprops=dict(color=C_BLUE, linewidth=2),
               manage_ticks=False)

    probe_vals = []
    for i in intensities:
        rows = probe_df[probe_df['probe_intensity'] == i]['total_ops_sec'].values
        probe_vals.append(rows[0] if len(rows) > 0 else 0)
    ax.plot(intensities, probe_vals, '-o', color=C_RED, linewidth=2, markersize=6, label='Probe worker')

    ax.set_xlabel('Probe Intensity')
    ax.set_ylabel('Per-Worker Throughput (ops/sec)')
    ax.set_title(f'Intensity Sweep — Base Workers (box) vs Probe Worker')
    ax.legend()
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'per_worker_intensity.png'))


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
            if name.startswith('per_worker_'):
                if 'intensity_sweep' in name:
                    plot_per_worker_intensity_sweep(csv_path)
                elif 'proc_slack' in name:
                    plot_per_worker_proc_slack(csv_path)
                else:
                    plot_per_worker_saturation(csv_path)
            elif 'throughput_vs_threads' in name or 'throughput_vs_workers' in name:
                plot_saturation(csv_path)
            elif name.startswith('slack_'):
                plot_slack(csv_path)
            elif name.startswith('proc_slack_'):
                plot_proc_slack(csv_path)
            else:
                print(f"  Skipping unknown format: {name}")
        except Exception as e:
            print(f"  Error: {e}")

    print("\nDone!")


if __name__ == '__main__':
    main()

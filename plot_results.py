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
from matplotlib.ticker import MaxNLocator
from pathlib import Path

# Colorblind-friendly palette (Wong 2011)
C_BLUE = '#0072B2'
C_ORANGE = '#E69F00'
C_GREEN = '#009E73'
C_RED = '#D55E00'
C_CYAN = '#56B4E9'
C_PURPLE = '#CC79A7'


def format_number(x):
    """Format large numbers with K/M suffixes."""
    if x >= 1_000_000:
        return f'{x/1_000_000:.1f}M'
    elif x >= 1_000:
        return f'{x/1_000:.1f}K'
    else:
        return f'{x:.0f}'


def save_fig(fig, path):
    """Save figure to path with tight layout, close it, and print the filename."""
    if not fig.get_constrained_layout():
        fig.tight_layout(pad=0.5, h_pad=None, w_pad=None, rect=(0, 0.02, 1, 1))
    fig.savefig(path, dpi=150)
    plt.close(fig)
    print(f"    {Path(path).name}")


def detect_experiment(name):
    """Return (label, mode, mode_plural, x_label) from CSV stem name."""
    is_cpu = 'cpu' in name.lower()
    is_proc = 'proc' in name.lower() or 'worker' in name.lower()
    label = 'CPU' if is_cpu else 'I/O'
    mode = 'Process' if is_proc else 'Thread'
    mode_plural = 'Processes' if is_proc else 'Threads'
    x_label = 'Worker Count' if is_proc else 'Thread Count'
    return label, mode, mode_plural, x_label


def plot_saturation(csv_path: str):
    """Generate graphs alongside a saturation CSV."""
    df = pd.read_csv(csv_path)
    name = Path(csv_path).stem
    folder = str(Path(csv_path).parent)
    label, mode, mode_plural, x_label = detect_experiment(name)
    x = df['workers']

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
    peak_label = f'Peak: {df.loc[peak_idx, "workers"]} {mode_plural.lower()}'
    if has_split:
        has_cpu = df['cpu_ops_sec'].max() > 0
        has_io = df['io_ops_sec'].max() > 0
        both = has_cpu and has_io

        if both:
            # Stacked subplots for mixed workloads
            fig, (ax_cpu, ax_io) = plt.subplots(2, 1, figsize=(10, 5), sharex=True,
                                                 layout='constrained')
            ax_cpu.errorbar(x, df['cpu_ops_sec'],
                            yerr=df['cpu_ops_stddev'] if has_cpu_stddev else None,
                            fmt='-o', color=C_BLUE, linewidth=2, markersize=6,
                            capsize=3, capthick=1, label='CPU ops/s')
            peak_x = df.loc[peak_idx, 'workers']
            ax_cpu.axvline(x=peak_x, color=C_RED, linestyle='--', alpha=0.7, label=peak_label)
            if primary_col == 'cpu_ops_sec':
                ax_cpu.scatter([peak_x], [df.loc[peak_idx, primary_col]],
                               color=C_RED, s=100, zorder=5)
            ax_cpu.set_ylabel('CPU ops/sec')
            ax_cpu.set_ylim(bottom=0)
            ax_cpu.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
            ax_cpu.legend(loc='best', fontsize=9)
            ax_cpu.set_title(f'{label} Saturation ({mode_plural}) — Throughput')
            ax_cpu.grid(True, alpha=0.3)

            ax_io.errorbar(x, df['io_ops_sec'],
                           yerr=df['io_ops_stddev'] if has_io_stddev else None,
                           fmt='-s', color=C_GREEN, linewidth=2, markersize=6,
                           capsize=3, capthick=1, label='IO ops/s')
            ax_io.axvline(x=peak_x, color=C_RED, linestyle='--', alpha=0.7)
            if primary_col == 'io_ops_sec':
                ax_io.scatter([peak_x], [df.loc[peak_idx, primary_col]],
                              color=C_RED, s=100, zorder=5)
            ax_io.set_xlabel(x_label)
            ax_io.set_ylabel('IO ops/sec')
            ax_io.set_ylim(bottom=0)
            ax_io.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
            ax_io.legend(loc='best', fontsize=9)
            ax_io.set_xlim(left=1)
            ax_io.xaxis.set_major_locator(MaxNLocator(integer=True))
            ax_io.grid(True, alpha=0.3)
        else:
            # Single axis for pure CPU or pure IO workloads
            col = 'cpu_ops_sec' if has_cpu else 'io_ops_sec'
            col_label = 'CPU ops/s' if has_cpu else 'IO ops/s'
            stddev = df['cpu_ops_stddev'] if (has_cpu and has_cpu_stddev) else (df['io_ops_stddev'] if has_io_stddev else None)

            fig, ax = plt.subplots(figsize=(8, 5))
            ax.errorbar(x, df[col], yerr=stddev, fmt='-o', color=C_BLUE,
                        linewidth=2, markersize=6, capsize=3, capthick=1, label=col_label)
            ax.axvline(x=df.loc[peak_idx, 'workers'], color=C_RED, linestyle='--', alpha=0.7,
                       label=peak_label)
            ax.scatter([df.loc[peak_idx, 'workers']], [df.loc[peak_idx, primary_col]],
                       color=C_RED, s=100, zorder=5)
            ax.set_xlabel(x_label)
            ax.set_ylabel(f'{col_label} (ops/sec)')
            ax.set_title(f'{label} Saturation ({mode_plural}) — Throughput')
            ax.set_ylim(bottom=0)
            ax.set_xlim(left=1)
            ax.xaxis.set_major_locator(MaxNLocator(integer=True))
            ax.legend()
            ax.grid(True, alpha=0.3)
            ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    else:
        fig, ax = plt.subplots(figsize=(8, 5))
        yerr = df.get('throughput_stddev')
        ax.errorbar(x, df['total_ops'], yerr=yerr, fmt='-o', color=C_BLUE,
                    linewidth=2, markersize=6, capsize=3, capthick=1)
        ax.axvline(x=df.loc[peak_idx, 'workers'], color=C_RED, linestyle='--', alpha=0.7,
                   label=peak_label)
        ax.scatter([df.loc[peak_idx, 'workers']], [df.loc[peak_idx, primary_col]],
                   color=C_RED, s=100, zorder=5)
        ax.set_xlabel(x_label)
        ax.set_ylabel('Throughput (ops/sec)')
        ax.set_title(f'{label} Saturation ({mode_plural}) — Throughput')
        ax.set_ylim(bottom=0)
        ax.set_xlim(left=1)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend()
        ax.grid(True, alpha=0.3)
        ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'throughput_total.png'))

    # 2. Per-worker throughput
    fig, ax = plt.subplots(figsize=(8, 5))
    ax.plot(x, df['throughput_per_worker'], '-s', color=C_BLUE,
            linewidth=2, markersize=6)
    first_tp = df.loc[0, 'throughput_per_worker']
    last_tp = df.loc[len(df)-1, 'throughput_per_worker']
    drop = (1 - last_tp / first_tp) * 100
    ax.annotate(f'{drop:.0f}% efficiency loss',
                xy=(x.iloc[-1], last_tp),
                xytext=(x.iloc[-1] * 0.7, (first_tp + last_tp) / 2),
                arrowprops=dict(arrowstyle='->', color='gray'),
                fontsize=10, color='gray')
    ax.set_xlabel(x_label)
    ax.set_ylabel(f'Throughput per {mode.lower()} (ops/sec)')
    ax.set_title(f'{label} Saturation ({mode_plural}) — Per-{mode} Efficiency')
    ax.set_ylim(bottom=0)
    ax.set_xlim(left=1)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'throughput_per_worker.png'))

    # 3. Delta throughput (marginal gain per additional worker)
    primary = df[primary_col]
    delta = primary.diff()
    # First point has no delta; skip it
    fig, ax = plt.subplots(figsize=(8, 5))
    colors = [C_BLUE if d >= 0 else C_RED for d in delta.iloc[1:]]
    ax.bar(x.iloc[1:], delta.iloc[1:], color=colors, alpha=0.7, width=0.8)
    ax.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
    # Mark where delta first goes negative (saturation onset)
    neg_idx = delta[delta < 0].first_valid_index()
    if neg_idx is not None:
        ax.axvline(x=df.loc[neg_idx, 'workers'], color=C_RED, linestyle='--', alpha=0.7,
                   label=f'First negative at {df.loc[neg_idx, "workers"]} {mode_plural.lower()}')
        ax.legend(loc='best', fontsize=9)
    ax.set_xlabel(x_label)
    ax.set_ylabel(f'\u0394 {primary_label} (ops/sec)')
    ax.set_title(f'{label} Saturation ({mode_plural}) \u2014 Marginal Throughput Gain')
    ax.set_xlim(left=1)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'delta_throughput.png'))

    # 4. Scaling efficiency: T(p) / (p * T(1))
    t1 = primary.iloc[0]
    if t1 > 0:
        efficiency = primary / (x * t1) * 100
        fig, ax = plt.subplots(figsize=(8, 5))
        ax.plot(x, efficiency, '-o', color=C_BLUE, linewidth=2, markersize=6)
        ax.axhline(y=100, color='gray', linestyle='--', alpha=0.5, label='Ideal linear scaling')
        ax.fill_between(x, efficiency, 100,
                        where=(efficiency <= 100), interpolate=True, alpha=0.15, color=C_RED)
        ax.fill_between(x, efficiency, 100,
                        where=(efficiency > 100), interpolate=True, alpha=0.15, color=C_GREEN)
        ax.set_xlabel(x_label)
        ax.set_ylabel('Scaling Efficiency (%)')
        ax.set_title(f'{label} Saturation ({mode_plural}) \u2014 Scaling Efficiency T(p) / (p \u00d7 T(1))')
        ax.set_ylim(bottom=0)
        ax.set_xlim(left=1)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend(loc='best', fontsize=9)
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'scaling_efficiency.png'))

    # 5. CPU & IO utilization vs workers
    if 'cpu_pct' in df.columns:
        fig, ax = plt.subplots(figsize=(8, 5))
        ax.plot(x, df['cpu_pct'], '-o', color=C_ORANGE, linewidth=2, markersize=6, label='CPU %')
        if 'io_util_pct' in df.columns:
            ax.plot(x, df['io_util_pct'], '-s', color=C_GREEN, linewidth=2, markersize=6, label='IO BW %')
        if 'io_iops_util_pct' in df.columns:
            ax.plot(x, df['io_iops_util_pct'], '-^', color=C_CYAN, linewidth=2, markersize=6, label='IO IOPS %')
        ax.set_xlabel(x_label)
        ax.set_ylabel('Utilization (%)')
        ax.set_title(f'{label} Saturation ({mode_plural}) — Resource Utilization')
        ax.set_ylim(0, 100)
        ax.set_xlim(left=1)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend()
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'utilization.png'))

    # 6. Combined: primary throughput metric + CPU% on dual axes
    if 'cpu_pct' in df.columns:
        fig, ax1 = plt.subplots(figsize=(8, 5))
        line_throughput = ax1.plot(x, df[primary_col], '-o', color=C_BLUE, linewidth=2,
                                   markersize=6, label=primary_label)
        ax1.set_xlabel(x_label)
        ax1.set_ylabel(f'{primary_label} (ops/sec)', color=C_BLUE)
        ax1.tick_params(axis='y', labelcolor=C_BLUE)
        ax1.set_ylim(bottom=0)
        ax1.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

        ax2 = ax1.twinx()
        line_cpu = ax2.plot(x, df['cpu_pct'], '-s', color=C_ORANGE, linewidth=2,
                            markersize=5, label='CPU %')
        ax2.set_ylabel('CPU Utilization (%)', color=C_ORANGE)
        ax2.tick_params(axis='y', labelcolor=C_ORANGE)
        ax2.set_ylim(bottom=0)

        lines = line_throughput + line_cpu
        labels = [l.get_label() for l in lines]
        ax1.legend(lines, labels, loc='best')
        ax1.set_title(f'{label} Saturation ({mode_plural}) — Throughput vs CPU')
        ax1.set_xlim(left=1)
        ax1.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax1.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'throughput_vs_cpu.png'))

    # 7. Throughput (% of peak) vs utilization split by resource (CPU subplot + IO subplot)
    if 'cpu_pct' in df.columns:
        has_io_util = 'io_util_pct' in df.columns and df['io_util_pct'].max() > 1
        has_iops_util = 'io_iops_util_pct' in df.columns and df['io_iops_util_pct'].max() > 1

        if has_split:
            cpu_tp = df['cpu_ops_sec']
            io_tp = df['io_ops_sec']
            has_cpu_tp = cpu_tp.max() > 0
            has_io_tp = io_tp.max() > 0
        else:
            cpu_tp = df['total_ops']
            io_tp = None
            has_cpu_tp = cpu_tp.max() > 0
            has_io_tp = False

        nplots = (1 if has_cpu_tp else 0) + (1 if (has_io_tp or has_io_util or has_iops_util) else 0)
        if nplots > 0:
            fig, axes = plt.subplots(nplots, 1, figsize=(10, 3 * nplots), sharex=True,
                                     layout='constrained')
            if nplots == 1:
                axes = [axes]
            ax_idx = 0

            if has_cpu_tp:
                ax_cpu = axes[ax_idx]
                ax_idx += 1
                cpu_pct_peak = cpu_tp / cpu_tp.max() * 100
                ax_cpu.plot(x, cpu_pct_peak, '-o', color=C_BLUE, linewidth=2,
                            markersize=5, label='CPU throughput (% of peak)')
                ax_cpu.plot(x, df['cpu_pct'], '--s', color=C_ORANGE, linewidth=1.5,
                            markersize=4, alpha=0.8, label='CPU utilization %')
                ax_cpu.set_ylabel('CPU %')
                ax_cpu.set_ylim(0, 105)
                ax_cpu.set_xlim(left=1)
                ax_cpu.grid(True, alpha=0.3)
                ax_cpu.legend(loc='best', fontsize=9)
                ax_cpu.set_title(f'{label} Saturation ({mode_plural}) — CPU')

            if has_io_tp or has_io_util or has_iops_util:
                ax_io = axes[ax_idx]
                if has_io_tp:
                    io_pct_peak = io_tp / io_tp.max() * 100
                    ax_io.plot(x, io_pct_peak, '-o', color=C_GREEN, linewidth=2,
                               markersize=5, label='IO throughput (% of peak)')
                if has_io_util:
                    ax_io.plot(x, df['io_util_pct'], '--s', color=C_CYAN, linewidth=1.5,
                               markersize=4, alpha=0.8, label='IO BW util %')
                if has_iops_util:
                    ax_io.plot(x, df['io_iops_util_pct'], '--^', color=C_PURPLE, linewidth=1.5,
                               markersize=4, alpha=0.8, label='IO IOPS util %')
                ax_io.set_ylabel('IO %')
                ax_io.set_ylim(0, 105)
                ax_io.set_xlim(left=1)
                ax_io.grid(True, alpha=0.3)
                ax_io.legend(loc='best', fontsize=9)
                if not has_cpu_tp:
                    ax_io.set_title(f'{label} Saturation ({mode_plural}) — IO')

            axes[-1].set_xlabel(x_label)
            axes[-1].xaxis.set_major_locator(MaxNLocator(integer=True))
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

    # 1. Throughput breakdown (stacked subplots)
    fig, (ax_cpu, ax_io) = plt.subplots(2, 1, figsize=(10, 5), sharex=True,
                                         layout='constrained')
    ax_cpu.errorbar(x, df['cpu_ops'],
                    yerr=df['cpu_ops_stddev'] if has_cpu_stddev else None,
                    fmt='o-', color=C_BLUE, label='CPU ops/s',
                    linewidth=2, markersize=5, capsize=3, capthick=1)
    ax_cpu.set_ylabel('CPU ops/sec')
    ax_cpu.set_ylim(bottom=0)
    ax_cpu.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    ax_cpu.legend(loc='best', fontsize=9)
    ax_cpu.set_title(f'{baseline_type} Slack — Throughput')
    ax_cpu.grid(True, alpha=0.3)

    ax_io.errorbar(x, df['io_ops'],
                   yerr=df['io_ops_stddev'] if has_io_stddev else None,
                   fmt='s-', color=C_GREEN, label='I/O ops/s',
                   linewidth=2, markersize=5, capsize=3, capthick=1)
    ax_io.set_xlabel('Extra Threads')
    ax_io.set_ylabel('I/O ops/sec')
    ax_io.set_ylim(bottom=0)
    ax_io.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    ax_io.legend(loc='best', fontsize=9)
    ax_io.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax_io.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'throughput.png'))

    # 2. Baseline degradation
    fig, ax = plt.subplots(figsize=(8, 5))
    ax.plot(x, df['baseline_change_pct'], '-o', color=C_RED, linewidth=2, markersize=5)
    ax.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
    ax.fill_between(x, df['baseline_change_pct'], 0,
                    where=(df['baseline_change_pct'] >= 0), interpolate=True, alpha=0.2, color=C_GREEN)
    ax.fill_between(x, df['baseline_change_pct'], 0,
                    where=(df['baseline_change_pct'] < 0), interpolate=True, alpha=0.2, color=C_RED)
    ax.set_xlabel('Extra Threads')
    ax.set_ylabel(f'{baseline_type} Baseline Change (%)')
    ax.set_title(f'{baseline_type} Slack — Baseline Degradation')
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'baseline_change.png'))

    # 3. CPU & IO utilization vs extra threads
    if 'cpu_pct' in df.columns:
        fig, ax = plt.subplots(figsize=(8, 5))
        ax.plot(x, df['cpu_pct'], '-o', color=C_ORANGE, linewidth=2, markersize=6, label='CPU %')
        if 'io_util_pct' in df.columns:
            ax.plot(x, df['io_util_pct'], '-s', color=C_GREEN, linewidth=2, markersize=6, label='IO BW %')
        if 'io_iops_util_pct' in df.columns:
            ax.plot(x, df['io_iops_util_pct'], '-^', color=C_CYAN, linewidth=2, markersize=6, label='IO IOPS %')
        ax.set_xlabel('Extra Threads')
        ax.set_ylabel('Utilization (%)')
        ax.set_title(f'{baseline_type} Slack — Resource Utilization')
        ax.set_ylim(0, 100)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend()
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'utilization.png'))

    # 4. Throughput (% of peak) vs utilization split by resource
    if 'cpu_pct' in df.columns:
        has_io_util = 'io_util_pct' in df.columns and df['io_util_pct'].max() > 1
        has_iops_util = 'io_iops_util_pct' in df.columns and df['io_iops_util_pct'].max() > 1
        has_cpu_tp = df['cpu_ops'].max() > 0
        has_io_tp = df['io_ops'].max() > 0

        nplots = (1 if has_cpu_tp else 0) + (1 if (has_io_tp or has_io_util or has_iops_util) else 0)
        if nplots > 0:
            fig, axes = plt.subplots(nplots, 1, figsize=(10, 3 * nplots), sharex=True,
                                     layout='constrained')
            if nplots == 1:
                axes = [axes]
            ax_idx = 0

            if has_cpu_tp:
                ax_cpu = axes[ax_idx]
                ax_idx += 1
                cpu_pct_peak = df['cpu_ops'] / df['cpu_ops'].max() * 100
                ax_cpu.plot(x, cpu_pct_peak, '-o', color=C_BLUE, linewidth=2,
                            markersize=5, label='CPU throughput (% of peak)')
                ax_cpu.plot(x, df['cpu_pct'], '--s', color=C_ORANGE, linewidth=1.5,
                            markersize=4, alpha=0.8, label='CPU utilization %')
                ax_cpu.set_ylabel('CPU %')
                ax_cpu.set_ylim(0, 105)
                ax_cpu.grid(True, alpha=0.3)
                ax_cpu.legend(loc='best', fontsize=9)
                ax_cpu.set_title(f'{baseline_type} Slack — CPU')

            if has_io_tp or has_io_util or has_iops_util:
                ax_io = axes[ax_idx]
                if has_io_tp:
                    io_pct_peak = df['io_ops'] / df['io_ops'].max() * 100
                    ax_io.plot(x, io_pct_peak, '-o', color=C_GREEN, linewidth=2,
                               markersize=5, label='IO throughput (% of peak)')
                if has_io_util:
                    ax_io.plot(x, df['io_util_pct'], '--s', color=C_CYAN, linewidth=1.5,
                               markersize=4, alpha=0.8, label='IO BW util %')
                if has_iops_util:
                    ax_io.plot(x, df['io_iops_util_pct'], '--^', color=C_PURPLE, linewidth=1.5,
                               markersize=4, alpha=0.8, label='IO IOPS util %')
                ax_io.set_ylabel('IO %')
                ax_io.set_ylim(0, 105)
                ax_io.grid(True, alpha=0.3)
                ax_io.legend(loc='best', fontsize=9)

            axes[-1].set_xlabel('Extra Threads')
            axes[-1].xaxis.set_major_locator(MaxNLocator(integer=True))
            save_fig(fig, os.path.join(folder, 'throughput_vs_utilization.png'))


def plot_per_worker_saturation(csv_path: str):
    """Box plot + fairness line for per-worker saturation CSVs."""
    df = pd.read_csv(csv_path)
    name = Path(csv_path).stem
    folder = str(Path(csv_path).parent)
    label, mode, mode_plural, x_label = detect_experiment(name)

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
    ax.set_title(f'{label} Saturation ({mode_plural}) — Per-Worker Distribution')
    ax.set_ylim(bottom=0)
    ax.set_xlim(left=1)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
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
    line_throughput = ax1.plot(worker_counts, totals, '-o', color=C_BLUE, linewidth=2, markersize=6, label='Total throughput')
    ax1.set_xlabel(x_label)
    ax1.set_ylabel('Total Throughput (ops/sec)', color=C_BLUE)
    ax1.tick_params(axis='y', labelcolor=C_BLUE)
    ax1.set_ylim(bottom=0)
    ax1.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

    ax2 = ax1.twinx()
    line_cv = ax2.plot(worker_counts, cvs, '-s', color=C_RED, linewidth=2, markersize=5, label='Fairness CV%')
    ax2.set_ylabel('Coefficient of Variation (%)', color=C_RED)
    ax2.tick_params(axis='y', labelcolor=C_RED)
    ax2.set_ylim(bottom=0)

    lines = line_throughput + line_cv
    ax1.legend(lines, [l.get_label() for l in lines], loc='best')
    ax1.set_title(f'{label} Saturation ({mode_plural}) — Throughput vs Fairness')
    ax1.set_xlim(left=1)
    ax1.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax1.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'per_worker_fairness.png'))


def plot_proc_slack(csv_path: str):
    """Generate graphs for proc slack CSVs."""
    df = pd.read_csv(csv_path)
    name = Path(csv_path).stem
    folder = str(Path(csv_path).parent)

    baseline_workers = int(df['baseline_workers'].iloc[0])
    # Parse baseline type from filename: e.g. "proc_slack_4io_proc_adding_50pct_io" → 'io' present → IO baseline
    is_io_baseline = 'io' in name.split('proc_slack_')[1].split('proc_adding')[0]
    baseline_label = 'I/O' if is_io_baseline else 'CPU'
    tracked_col = 'baseline_io_ops' if is_io_baseline else 'baseline_cpu_ops'
    x = df['extra_workers']

    print(f"  -> {folder}/")

    # 1. Baseline throughput degradation vs extra workers (stacked subplots)
    fig, (ax_cpu, ax_io) = plt.subplots(2, 1, figsize=(10, 5), sharex=True,
                                         layout='constrained')
    has_baseline_cpu = df['baseline_cpu_ops'].max() > 0
    has_extra_cpu = df['extra_cpu_ops'].max() > 0
    has_baseline_io = df['baseline_io_ops'].max() > 0
    has_extra_io = df['extra_io_ops'].max() > 0

    if has_baseline_cpu:
        ax_cpu.errorbar(x, df['baseline_cpu_ops'],
                        yerr=df['baseline_cpu_stddev'],
                        fmt='o-', color=C_BLUE, linewidth=2, markersize=5,
                        capsize=3, capthick=1, label=f'Baseline ({baseline_workers} procs)')
    if has_extra_cpu:
        ax_cpu.plot(x, df['extra_cpu_ops'], '--^', color=C_CYAN, linewidth=1.5,
                    markersize=4, alpha=0.8, label='Extra')
    ax_cpu.set_ylabel('CPU ops/sec')
    ax_cpu.set_ylim(bottom=0)
    ax_cpu.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    if has_baseline_cpu or has_extra_cpu:
        ax_cpu.legend(loc='best', fontsize=9)
    ax_cpu.set_title(f'Proc Slack ({baseline_label} baseline, {baseline_workers} procs) — Throughput')
    ax_cpu.grid(True, alpha=0.3)

    if has_baseline_io:
        ax_io.errorbar(x, df['baseline_io_ops'],
                       yerr=df['baseline_io_stddev'],
                       fmt='s-', color=C_GREEN, linewidth=2, markersize=5,
                       capsize=3, capthick=1, label=f'Baseline ({baseline_workers} procs)')
    if has_extra_io:
        ax_io.plot(x, df['extra_io_ops'], '--v', color=C_ORANGE, linewidth=1.5,
                   markersize=4, alpha=0.8, label='Extra')
    ax_io.set_xlabel('Extra Worker Processes')
    ax_io.set_ylabel('IO ops/sec')
    ax_io.set_ylim(bottom=0)
    ax_io.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    if has_baseline_io or has_extra_io:
        ax_io.legend(loc='best', fontsize=9)
    ax_io.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax_io.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'throughput.png'))

    # 2. Baseline change % with zero line
    fig, ax = plt.subplots(figsize=(8, 5))
    ax.plot(x, df['baseline_change_pct'], '-o', color=C_RED, linewidth=2, markersize=5)
    ax.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
    ax.fill_between(x, df['baseline_change_pct'], 0,
                    where=(df['baseline_change_pct'] >= 0), interpolate=True, alpha=0.2, color=C_GREEN)
    ax.fill_between(x, df['baseline_change_pct'], 0,
                    where=(df['baseline_change_pct'] < 0), interpolate=True, alpha=0.2, color=C_RED)
    ax.set_xlabel('Extra Worker Processes')
    ax.set_ylabel(f'{baseline_label} Baseline Change (%)')
    ax.set_title(f'Proc Slack — Baseline Degradation ({baseline_label} baseline, {baseline_workers} procs)')
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'baseline_change.png'))

    # 3. Utilization
    if 'cpu_pct' in df.columns:
        fig, ax = plt.subplots(figsize=(8, 5))
        ax.plot(x, df['cpu_pct'], '-o', color=C_ORANGE, linewidth=2, markersize=6, label='CPU %')
        if 'io_util_pct' in df.columns:
            ax.plot(x, df['io_util_pct'], '-s', color=C_GREEN, linewidth=2, markersize=6, label='IO BW %')
        if 'io_iops_util_pct' in df.columns:
            ax.plot(x, df['io_iops_util_pct'], '-^', color=C_CYAN, linewidth=2, markersize=6, label='IO IOPS %')
        ax.set_xlabel('Extra Worker Processes')
        ax.set_ylabel('Utilization (%)')
        ax.set_title(f'Proc Slack — Resource Utilization')
        ax.set_ylim(0, 100)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend()
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'utilization.png'))

    # 4. Throughput (% of peak) vs utilization split by resource
    if 'cpu_pct' in df.columns:
        has_io_util = 'io_util_pct' in df.columns and df['io_util_pct'].max() > 1
        has_iops_util = 'io_iops_util_pct' in df.columns and df['io_iops_util_pct'].max() > 1
        has_any_cpu = df['baseline_cpu_ops'].max() > 0 or df['extra_cpu_ops'].max() > 0
        has_any_io = df['baseline_io_ops'].max() > 0 or df['extra_io_ops'].max() > 0

        nplots = (1 if has_any_cpu else 0) + (1 if (has_any_io or has_io_util or has_iops_util) else 0)
        if nplots > 0:
            fig, axes = plt.subplots(nplots, 1, figsize=(10, 3 * nplots), sharex=True,
                                     layout='constrained')
            if nplots == 1:
                axes = [axes]
            ax_idx = 0

            if has_any_cpu:
                ax_cpu = axes[ax_idx]
                ax_idx += 1
                # Combine baseline + extra CPU throughput for peak normalization
                total_cpu = df['baseline_cpu_ops'] + df['extra_cpu_ops']
                cpu_peak = total_cpu.max()
                if cpu_peak > 0:
                    if df['baseline_cpu_ops'].max() > 0:
                        ax_cpu.plot(x, df['baseline_cpu_ops'] / cpu_peak * 100, '-o', color=C_BLUE,
                                    linewidth=2, markersize=5, label='Baseline CPU (% of peak)')
                    if df['extra_cpu_ops'].max() > 0:
                        ax_cpu.plot(x, df['extra_cpu_ops'] / cpu_peak * 100, '--^', color=C_CYAN,
                                    linewidth=1.5, markersize=4, alpha=0.8, label='Extra CPU (% of peak)')
                ax_cpu.plot(x, df['cpu_pct'], '--s', color=C_ORANGE, linewidth=1.5,
                            markersize=4, alpha=0.8, label='CPU utilization %')
                ax_cpu.set_ylabel('CPU %')
                ax_cpu.set_ylim(0, 105)
                ax_cpu.grid(True, alpha=0.3)
                ax_cpu.legend(loc='best', fontsize=9)
                ax_cpu.set_title(f'Proc Slack ({baseline_label} baseline, {baseline_workers} procs) — CPU')

            if has_any_io or has_io_util or has_iops_util:
                ax_io = axes[ax_idx]
                total_io = df['baseline_io_ops'] + df['extra_io_ops']
                io_peak = total_io.max()
                if io_peak > 0:
                    if df['baseline_io_ops'].max() > 0:
                        ax_io.plot(x, df['baseline_io_ops'] / io_peak * 100, '-o', color=C_GREEN,
                                   linewidth=2, markersize=5, label='Baseline IO (% of peak)')
                    if df['extra_io_ops'].max() > 0:
                        ax_io.plot(x, df['extra_io_ops'] / io_peak * 100, '--v', color=C_ORANGE,
                                   linewidth=1.5, markersize=4, alpha=0.8, label='Extra IO (% of peak)')
                if has_io_util:
                    ax_io.plot(x, df['io_util_pct'], '--s', color=C_CYAN, linewidth=1.5,
                               markersize=4, alpha=0.8, label='IO BW util %')
                if has_iops_util:
                    ax_io.plot(x, df['io_iops_util_pct'], '--^', color=C_PURPLE, linewidth=1.5,
                               markersize=4, alpha=0.8, label='IO IOPS util %')
                ax_io.set_ylabel('IO %')
                ax_io.set_ylim(0, 105)
                ax_io.grid(True, alpha=0.3)
                ax_io.legend(loc='best', fontsize=9)

            axes[-1].set_xlabel('Extra Worker Processes')
            axes[-1].xaxis.set_major_locator(MaxNLocator(integer=True))
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

    # 1. Box plot of baseline worker throughput distribution
    fig, ax = plt.subplots(figsize=(max(8, len(extra_counts) * 0.5), 5))
    base_data = [base_df[base_df['extra_workers'] == e]['total_ops_sec'].values for e in extra_counts]

    ax.boxplot(base_data, positions=extra_counts, widths=box_width * 0.7, patch_artist=True,
               boxprops=dict(facecolor=C_CYAN, alpha=0.7),
               medianprops=dict(color=C_BLUE, linewidth=2), manage_ticks=False)

    ax.set_xticks(extra_counts)
    ax.set_xticklabels(extra_counts)
    ax.set_xlabel('Extra Worker Processes')
    ax.set_ylabel('Per-Worker Throughput (ops/sec)')
    ax.set_title(f'Proc Slack — Baseline Worker Distribution ({baseline_workers} procs)')
    ax.set_ylim(bottom=0)
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
    line_throughput = ax1.plot(extra_counts, totals, '-o', color=C_BLUE, linewidth=2, markersize=6, label='Baseline total throughput')
    ax1.set_xlabel('Extra Worker Processes')
    ax1.set_ylabel('Baseline Total Throughput (ops/sec)', color=C_BLUE)
    ax1.tick_params(axis='y', labelcolor=C_BLUE)
    ax1.set_ylim(bottom=0)
    ax1.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

    ax2 = ax1.twinx()
    line_cv = ax2.plot(extra_counts, cvs, '-s', color=C_RED, linewidth=2, markersize=5, label='Baseline fairness CV%')
    ax2.set_ylabel('Coefficient of Variation (%)', color=C_RED)
    ax2.tick_params(axis='y', labelcolor=C_RED)
    ax2.set_ylim(bottom=0)

    lines = line_throughput + line_cv
    ax1.legend(lines, [l.get_label() for l in lines], loc='best')
    ax1.set_title(f'Proc Slack — Baseline Throughput vs Fairness')
    ax1.xaxis.set_major_locator(MaxNLocator(integer=True))
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
    ax.set_ylim(bottom=0)
    ax.legend()
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'per_worker_intensity.png'))


def plot_soi(csv_path: str):
    """Generate graphs for SoI (Service of Interest) sweep CSVs."""
    df = pd.read_csv(csv_path)
    name = Path(csv_path).stem
    folder = str(Path(csv_path).parent)

    soi_type = df['soi_type'].iloc[0]
    if soi_type == 'ioops':
        soi_type = 'iops'
    victim_workers = int(df['victim_workers'].iloc[0])
    x = df['soi_workers']

    has_cpu_stddev = 'victim_cpu_stddev' in df.columns
    has_io_stddev = 'victim_io_stddev' in df.columns

    # Derive split change pcts from raw ops if not in CSV
    if 'victim_cpu_change_pct' not in df.columns and 'victim_cpu_ops' in df.columns:
        base_cpu = df['victim_cpu_ops'].iloc[0]
        base_io = df['victim_io_ops'].iloc[0]
        df['victim_cpu_change_pct'] = ((df['victim_cpu_ops'] - base_cpu) / base_cpu * 100.0) if base_cpu > 0 else 0.0
        df['victim_io_change_pct'] = ((df['victim_io_ops'] - base_io) / base_io * 100.0) if base_io > 0 else 0.0

    print(f"  -> {folder}/")

    # 1. Victim throughput vs SoI workers (stacked CPU + IO subplots)
    has_cpu = df['victim_cpu_ops'].max() > 0
    has_io = df['victim_io_ops'].max() > 0
    nplots = (1 if has_cpu else 0) + (1 if has_io else 0)
    if nplots == 0:
        nplots = 1

    if nplots == 2:
        fig, (ax_cpu, ax_io) = plt.subplots(2, 1, figsize=(10, 5), sharex=True,
                                             layout='constrained')
        ax_cpu.errorbar(x, df['victim_cpu_ops'],
                        yerr=df['victim_cpu_stddev'] if has_cpu_stddev else None,
                        fmt='-o', color=C_BLUE, linewidth=2, markersize=6,
                        capsize=3, capthick=1, label='Victim CPU ops/s')
        ax_cpu.set_ylabel('CPU ops/sec')
        ax_cpu.set_ylim(bottom=0)
        ax_cpu.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
        ax_cpu.legend(loc='best', fontsize=9)
        ax_cpu.set_title(f'SoI Sweep ({soi_type}) — Victim Throughput ({victim_workers} victims)')
        ax_cpu.grid(True, alpha=0.3)

        ax_io.errorbar(x, df['victim_io_ops'],
                       yerr=df['victim_io_stddev'] if has_io_stddev else None,
                       fmt='-s', color=C_GREEN, linewidth=2, markersize=6,
                       capsize=3, capthick=1, label='Victim IO ops/s')
        ax_io.set_xlabel('SoI Workers')
        ax_io.set_ylabel('IO ops/sec')
        ax_io.set_ylim(bottom=0)
        ax_io.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
        ax_io.legend(loc='best', fontsize=9)
        ax_io.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax_io.grid(True, alpha=0.3)
    else:
        fig, ax = plt.subplots(figsize=(8, 5))
        col = 'victim_cpu_ops' if has_cpu else 'victim_io_ops'
        col_label = 'Victim CPU ops/s' if has_cpu else 'Victim IO ops/s'
        stddev = df['victim_cpu_stddev'] if (has_cpu and has_cpu_stddev) else (df['victim_io_stddev'] if has_io_stddev else None)
        ax.errorbar(x, df[col], yerr=stddev, fmt='-o', color=C_BLUE,
                    linewidth=2, markersize=6, capsize=3, capthick=1, label=col_label)
        ax.set_xlabel('SoI Workers')
        ax.set_ylabel(f'{col_label} (ops/sec)')
        ax.set_title(f'SoI Sweep ({soi_type}) — Victim Throughput ({victim_workers} victims)')
        ax.set_ylim(bottom=0)
        ax.set_xlim(left=0)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend(loc='best', fontsize=9)
        ax.grid(True, alpha=0.3)
        ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'victim_throughput.png'))

    # 2. Victim degradation percentage (split CPU/IO if both present)
    has_cpu_change = 'victim_cpu_change_pct' in df.columns and has_cpu
    has_io_change = 'victim_io_change_pct' in df.columns and has_io
    if has_cpu_change and has_io_change:
        fig, (ax_cpu, ax_io) = plt.subplots(2, 1, figsize=(8, 7), sharex=True, layout='constrained')
        ax_cpu.plot(x, df['victim_cpu_change_pct'], '-o', color=C_BLUE, linewidth=2, markersize=5)
        ax_cpu.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
        ax_cpu.fill_between(x, df['victim_cpu_change_pct'], 0,
                            where=(df['victim_cpu_change_pct'] < 0), interpolate=True, alpha=0.2, color=C_RED)
        ax_cpu.set_ylabel('CPU Change (%)')
        ax_cpu.grid(True, alpha=0.3)

        ax_io.plot(x, df['victim_io_change_pct'], '-s', color=C_GREEN, linewidth=2, markersize=5)
        ax_io.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
        ax_io.fill_between(x, df['victim_io_change_pct'], 0,
                           where=(df['victim_io_change_pct'] < 0), interpolate=True, alpha=0.2, color=C_RED)
        ax_io.set_xlabel('SoI Workers')
        ax_io.set_ylabel('IO Change (%)')
        ax_io.grid(True, alpha=0.3)
        fig.suptitle(f'SoI Sweep ({soi_type}) — Victim Degradation ({victim_workers} victims)')
    else:
        fig, ax = plt.subplots(figsize=(8, 5))
        ax.plot(x, df['victim_change_pct'], '-o', color=C_RED, linewidth=2, markersize=5)
        ax.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
        ax.fill_between(x, df['victim_change_pct'], 0,
                        where=(df['victim_change_pct'] >= 0), interpolate=True, alpha=0.2, color=C_GREEN)
        ax.fill_between(x, df['victim_change_pct'], 0,
                        where=(df['victim_change_pct'] < 0), interpolate=True, alpha=0.2, color=C_RED)
        ax.set_xlabel('SoI Workers')
        ax.set_ylabel('Victim Throughput Change (%)')
        ax.set_title(f'SoI Sweep ({soi_type}) — Victim Degradation ({victim_workers} victims)')
        ax.set_xlim(left=0)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'victim_degradation.png'))

    # 3. SoI throughput
    if df['soi_ops'].max() > 0:
        fig, ax = plt.subplots(figsize=(8, 5))
        ax.plot(x, df['soi_ops'], '-^', color=C_ORANGE, linewidth=2, markersize=6, label='SoI ops/s')
        ax.set_xlabel('SoI Workers')
        ax.set_ylabel('SoI Throughput (ops/sec)')
        ax.set_title(f'SoI Sweep ({soi_type}) — SoI Throughput')
        ax.set_ylim(bottom=0)
        ax.set_xlim(left=0)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend(loc='best', fontsize=9)
        ax.grid(True, alpha=0.3)
        ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
        save_fig(fig, os.path.join(folder, 'soi_throughput.png'))

    # 4. Combined: victim throughput + degradation on dual axes
    primary_col = 'victim_io_ops' if has_io else 'victim_cpu_ops'
    primary_label = 'Victim IO ops/s' if has_io else 'Victim CPU ops/s'
    fig, ax1 = plt.subplots(figsize=(8, 5))
    line_tp = ax1.plot(x, df[primary_col], '-o', color=C_BLUE, linewidth=2,
                       markersize=6, label=primary_label)
    ax1.set_xlabel('SoI Workers')
    ax1.set_ylabel(f'{primary_label} (ops/sec)', color=C_BLUE)
    ax1.tick_params(axis='y', labelcolor=C_BLUE)
    ax1.set_ylim(bottom=0)
    ax1.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

    ax2 = ax1.twinx()
    line_deg = ax2.plot(x, df['victim_change_pct'], '-s', color=C_RED, linewidth=2,
                        markersize=5, label='Victim change %')
    ax2.set_ylabel('Victim Change (%)', color=C_RED)
    ax2.tick_params(axis='y', labelcolor=C_RED)

    lines = line_tp + line_deg
    ax1.legend(lines, [l.get_label() for l in lines], loc='best')
    ax1.set_title(f'SoI Sweep ({soi_type}) — Throughput vs Degradation')
    ax1.set_xlim(left=0)
    ax1.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax1.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'throughput_vs_degradation.png'))

    # 5. Resource utilization
    if 'cpu_pct' in df.columns:
        fig, ax = plt.subplots(figsize=(8, 5))
        ax.plot(x, df['cpu_pct'], '-o', color=C_ORANGE, linewidth=2, markersize=6, label='CPU %')
        if 'io_util_pct' in df.columns:
            ax.plot(x, df['io_util_pct'], '-s', color=C_GREEN, linewidth=2, markersize=6, label='IO BW %')
        if 'io_iops_util_pct' in df.columns:
            ax.plot(x, df['io_iops_util_pct'], '-^', color=C_CYAN, linewidth=2, markersize=6, label='IO IOPS %')
        ax.set_xlabel('SoI Workers')
        ax.set_ylabel('Utilization (%)')
        ax.set_title(f'SoI Sweep ({soi_type}) — Resource Utilization')
        ax.set_ylim(0, 100)
        ax.set_xlim(left=0)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend()
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'utilization.png'))

    # 6. PSI pressure
    has_psi = 'psi_cpu_some_us' in df.columns
    if has_psi:
        fig, ax = plt.subplots(figsize=(8, 5))
        if 'io_psi_pct' in df.columns:
            ax.plot(x, df['io_psi_pct'], '-s', color=C_GREEN, linewidth=2, markersize=6, label='IO PSI %')
        ax.set_xlabel('SoI Workers')
        ax.set_ylabel('PSI Pressure (%)')
        ax.set_title(f'SoI Sweep ({soi_type}) — Pressure Stall Information')
        ax.set_ylim(bottom=0)
        ax.set_xlim(left=0)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend()
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'psi_pressure.png'))

    # 7. Throughput vs utilization (stacked CPU + IO subplots with twin axes)
    if 'cpu_pct' in df.columns and has_cpu and has_io:
        has_io_util = 'io_util_pct' in df.columns
        has_iops_util = 'io_iops_util_pct' in df.columns

        fig, (ax_cpu, ax_io) = plt.subplots(2, 1, figsize=(10, 6), sharex=True,
                                             layout='constrained')

        # CPU subplot: victim CPU throughput + CPU utilization
        ax_cpu.errorbar(x, df['victim_cpu_ops'],
                        yerr=df['victim_cpu_stddev'] if has_cpu_stddev else None,
                        fmt='-o', color=C_BLUE, linewidth=2, markersize=5,
                        capsize=3, capthick=1, label='Victim CPU ops/s')
        ax_cpu.set_ylabel('CPU ops/sec', color=C_BLUE)
        ax_cpu.tick_params(axis='y', labelcolor=C_BLUE)
        ax_cpu.set_ylim(bottom=0)
        ax_cpu.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
        ax_cpu.grid(True, alpha=0.3)

        ax_cpu2 = ax_cpu.twinx()
        ax_cpu2.plot(x, df['cpu_pct'], '--s', color=C_ORANGE, linewidth=1.5,
                     markersize=4, alpha=0.8, label='CPU util %')
        ax_cpu2.set_ylabel('CPU Utilization (%)', color=C_ORANGE)
        ax_cpu2.tick_params(axis='y', labelcolor=C_ORANGE)
        ax_cpu2.set_ylim(0, 105)

        lines_cpu = ax_cpu.get_lines() + ax_cpu2.get_lines()
        # errorbar returns a container, not a Line2D, so collect legend manually
        labels_cpu = ['Victim CPU ops/s', 'CPU util %']
        ax_cpu.legend(lines_cpu[-1:], labels_cpu[-1:], loc='best', fontsize=9)
        from matplotlib.lines import Line2D
        handles_cpu = [Line2D([], [], color=C_BLUE, marker='o', markersize=5, label='Victim CPU ops/s'),
                       Line2D([], [], color=C_ORANGE, marker='s', markersize=4, linestyle='--', alpha=0.8, label='CPU util %')]
        ax_cpu.legend(handles=handles_cpu, loc='best', fontsize=9)
        ax_cpu.set_title(f'SoI Sweep ({soi_type}) — Throughput vs Utilization ({victim_workers} victims)')

        # IO subplot: victim IO throughput + IO BW/IOPS utilization
        ax_io.errorbar(x, df['victim_io_ops'],
                       yerr=df['victim_io_stddev'] if has_io_stddev else None,
                       fmt='-o', color=C_GREEN, linewidth=2, markersize=5,
                       capsize=3, capthick=1, label='Victim IO ops/s')
        ax_io.set_ylabel('IO ops/sec', color=C_GREEN)
        ax_io.tick_params(axis='y', labelcolor=C_GREEN)
        ax_io.set_ylim(bottom=0)
        ax_io.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
        ax_io.set_xlabel('SoI Workers')
        ax_io.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax_io.grid(True, alpha=0.3)

        ax_io2 = ax_io.twinx()
        handles_io = [Line2D([], [], color=C_GREEN, marker='o', markersize=5, label='Victim IO ops/s')]
        if has_io_util:
            ax_io2.plot(x, df['io_util_pct'], '--s', color=C_CYAN, linewidth=1.5,
                        markersize=4, alpha=0.8, label='IO BW util %')
            handles_io.append(Line2D([], [], color=C_CYAN, marker='s', markersize=4, linestyle='--', alpha=0.8, label='IO BW util %'))
        if has_iops_util:
            ax_io2.plot(x, df['io_iops_util_pct'], '--^', color=C_PURPLE, linewidth=1.5,
                        markersize=4, alpha=0.8, label='IO IOPS util %')
            handles_io.append(Line2D([], [], color=C_PURPLE, marker='^', markersize=4, linestyle='--', alpha=0.8, label='IO IOPS util %'))
        ax_io2.set_ylabel('IO Utilization (%)')
        ax_io2.set_ylim(0, 105)
        ax_io.legend(handles=handles_io, loc='best', fontsize=9)

        save_fig(fig, os.path.join(folder, 'throughput_vs_utilization.png'))


def plot_per_worker_soi(csv_path: str):
    """Box plot of victim worker throughput distribution as SoI workers increase."""
    df = pd.read_csv(csv_path)
    folder = str(Path(csv_path).parent)

    soi_counts = sorted(df['soi_workers'].unique())
    victim_df = df[~df['is_soi']]

    print(f"  -> {folder}/")

    box_width = (soi_counts[1] - soi_counts[0]) * 0.6 if len(soi_counts) > 1 else 0.6

    # 1. Box plot of victim worker throughput distribution
    fig, ax = plt.subplots(figsize=(max(8, len(soi_counts) * 0.5), 5))
    data = [victim_df[victim_df['soi_workers'] == s]['total_ops_sec'].values for s in soi_counts]
    ax.boxplot(data, positions=soi_counts, widths=box_width * 0.7, patch_artist=True,
               boxprops=dict(facecolor=C_CYAN, alpha=0.7),
               medianprops=dict(color=C_BLUE, linewidth=2), manage_ticks=False)
    ax.set_xticks(soi_counts)
    ax.set_xticklabels(soi_counts)
    ax.set_xlabel('SoI Workers')
    ax.set_ylabel('Per-Worker Throughput (ops/sec)')
    ax.set_title('SoI Sweep — Victim Worker Distribution')
    ax.set_ylim(bottom=0)
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'per_worker_distribution.png'))

    # 2. Victim total throughput + CV% (fairness)
    totals = [victim_df[victim_df['soi_workers'] == s]['total_ops_sec'].sum() for s in soi_counts]
    cvs = []
    for s in soi_counts:
        vals = victim_df[victim_df['soi_workers'] == s]['total_ops_sec'].values
        mean = vals.mean()
        cvs.append(vals.std() / mean * 100 if mean > 0 else 0)

    fig, ax1 = plt.subplots(figsize=(8, 5))
    line_tp = ax1.plot(soi_counts, totals, '-o', color=C_BLUE, linewidth=2, markersize=6, label='Victim total throughput')
    ax1.set_xlabel('SoI Workers')
    ax1.set_ylabel('Victim Total Throughput (ops/sec)', color=C_BLUE)
    ax1.tick_params(axis='y', labelcolor=C_BLUE)
    ax1.set_ylim(bottom=0)
    ax1.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

    ax2 = ax1.twinx()
    line_cv = ax2.plot(soi_counts, cvs, '-s', color=C_RED, linewidth=2, markersize=5, label='Victim fairness CV%')
    ax2.set_ylabel('Coefficient of Variation (%)', color=C_RED)
    ax2.tick_params(axis='y', labelcolor=C_RED)
    ax2.set_ylim(bottom=0)

    lines = line_tp + line_cv
    ax1.legend(lines, [l.get_label() for l in lines], loc='best')
    ax1.set_title('SoI Sweep — Victim Throughput vs Fairness')
    ax1.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax1.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'per_worker_fairness.png'))


def plot_ext_timeseries(csv_path: str):
    """Generate time-series plots for external workload SoI sweep."""
    df = pd.read_csv(csv_path)
    name = Path(csv_path).stem
    folder = str(Path(csv_path).parent)

    # Extract SoI type from filename: timeseries_ext_soi_<type>
    soi_type = name.replace('timeseries_ext_soi_', '')

    soi_counts = sorted(df['soi_workers'].unique())

    print(f"  -> {folder}/")

    # 1. External throughput over time, one line per SoI worker count
    fig, ax = plt.subplots(figsize=(10, 6))
    cmap = plt.cm.viridis
    for i, s in enumerate(soi_counts):
        sub = df[df['soi_workers'] == s]
        color = cmap(i / max(len(soi_counts) - 1, 1))
        ax.plot(sub['elapsed_ms'] / 1000, sub['ext_ops_sec'], '-', color=color,
                linewidth=1.2, alpha=0.8, label=f'{int(s)} SoI')
    ax.set_xlabel('Elapsed (seconds)')
    ax.set_ylabel('External Throughput (ops/sec)')
    ax.set_title(f'SoI Sweep ({soi_type}) — External Throughput Time Series')
    ax.set_ylim(bottom=0)
    ax.legend(loc='best', fontsize=8, ncol=2)
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'timeseries_ext_throughput.png'))

    # 2. Heatmap: external throughput over time × SoI workers
    if len(soi_counts) > 1:
        # Pivot to 2D grid
        pivot = df.pivot_table(index='soi_workers', columns='elapsed_ms', values='ext_ops_sec')
        fig, ax = plt.subplots(figsize=(12, 5))
        im = ax.imshow(pivot.values, aspect='auto', cmap='RdYlGn',
                       extent=[pivot.columns.min() / 1000, pivot.columns.max() / 1000,
                               pivot.index.max() + 0.5, pivot.index.min() - 0.5])
        ax.set_xlabel('Elapsed (seconds)')
        ax.set_ylabel('SoI Workers')
        ax.set_title(f'SoI Sweep ({soi_type}) — Throughput Heatmap')
        ax.set_yticks(soi_counts)
        cbar = fig.colorbar(im, ax=ax, label='External ops/sec')
        cbar.formatter = plt.FuncFormatter(lambda v, p: format_number(v))
        cbar.update_ticks()
        save_fig(fig, os.path.join(folder, 'timeseries_heatmap.png'))

    # 3. CPU utilization over time — small multiples (one row per SoI count)
    if 'cpu_pct' in df.columns:
        n = len(soi_counts)
        fig, axes = plt.subplots(n, 1, figsize=(12, 1.8 * n), sharex=True, sharey=True)
        if n == 1:
            axes = [axes]
        for i, (ax, s) in enumerate(zip(axes, soi_counts)):
            sub = df[df['soi_workers'] == s]
            color = cmap(i / max(n - 1, 1))
            ax.plot(sub['elapsed_ms'] / 1000, sub['cpu_pct'], '-', color=color,
                    linewidth=1.0, alpha=0.8)
            ax.set_ylabel(f'{int(s)} SoI', fontsize=9, rotation=0, labelpad=35, va='center')
            ax.set_ylim(0, 105)
            ax.grid(True, alpha=0.3)
            ax.tick_params(axis='y', labelsize=7)
        axes[-1].set_xlabel('Elapsed (seconds)')
        fig.suptitle(f'SoI Sweep ({soi_type}) — CPU Utilization Time Series', fontsize=12)
        fig.tight_layout()
        save_fig(fig, os.path.join(folder, 'timeseries_cpu.png'))

    # 3b. IO BW utilization over time — small multiples (one row per SoI count)
    if 'io_util_pct' in df.columns:
        n = len(soi_counts)
        fig, axes = plt.subplots(n, 1, figsize=(12, 1.8 * n), sharex=True, sharey=True)
        if n == 1:
            axes = [axes]
        for i, (ax, s) in enumerate(zip(axes, soi_counts)):
            sub = df[df['soi_workers'] == s]
            color = cmap(i / max(n - 1, 1))
            ax.plot(sub['elapsed_ms'] / 1000, sub['io_util_pct'], '-', color=color,
                    linewidth=1.0, alpha=0.8)
            ax.set_ylabel(f'{int(s)} SoI', fontsize=9, rotation=0, labelpad=35, va='center')
            ax.set_ylim(0, 105)
            ax.grid(True, alpha=0.3)
            ax.tick_params(axis='y', labelsize=7)
        axes[-1].set_xlabel('Elapsed (seconds)')
        fig.suptitle(f'SoI Sweep ({soi_type}) — IO BW Utilization Time Series', fontsize=12)
        fig.tight_layout()
        save_fig(fig, os.path.join(folder, 'timeseries_io_bw.png'))

    # 3c. IO IOPS utilization over time — small multiples (one row per SoI count)
    if 'io_iops_util_pct' in df.columns:
        n = len(soi_counts)
        y_max = max(df['io_iops_util_pct'].max() * 1.1, 10)
        fig, axes = plt.subplots(n, 1, figsize=(12, 1.8 * n), sharex=True, sharey=True)
        if n == 1:
            axes = [axes]
        for i, (ax, s) in enumerate(zip(axes, soi_counts)):
            sub = df[df['soi_workers'] == s]
            color = cmap(i / max(n - 1, 1))
            ax.plot(sub['elapsed_ms'] / 1000, sub['io_iops_util_pct'], '-', color=color,
                    linewidth=1.0, alpha=0.8)
            ax.set_ylabel(f'{int(s)} SoI', fontsize=9, rotation=0, labelpad=35, va='center')
            ax.set_ylim(0, y_max)
            ax.grid(True, alpha=0.3)
            ax.tick_params(axis='y', labelsize=7)
        axes[-1].set_xlabel('Elapsed (seconds)')
        fig.suptitle(f'SoI Sweep ({soi_type}) — IO IOPS Utilization Time Series', fontsize=12)
        fig.tight_layout()
        save_fig(fig, os.path.join(folder, 'timeseries_io_iops.png'))

    # 3d. Memory utilization over time — small multiples (one row per SoI count)
    if 'mem_usage_pct' in df.columns:
        n = len(soi_counts)
        fig, axes = plt.subplots(n, 1, figsize=(12, 1.8 * n), sharex=True, sharey=True)
        if n == 1:
            axes = [axes]
        for i, (ax, s) in enumerate(zip(axes, soi_counts)):
            sub = df[df['soi_workers'] == s]
            color = cmap(i / max(n - 1, 1))
            ax.plot(sub['elapsed_ms'] / 1000, sub['mem_usage_pct'], '-', color=color,
                    linewidth=1.0, alpha=0.8)
            ax.set_ylabel(f'{int(s)} SoI', fontsize=9, rotation=0, labelpad=35, va='center')
            ax.set_ylim(0, 105)
            ax.grid(True, alpha=0.3)
            ax.tick_params(axis='y', labelsize=7)
        axes[-1].set_xlabel('Elapsed (seconds)')
        fig.suptitle(f'SoI Sweep ({soi_type}) — Memory Utilization Time Series', fontsize=12)
        fig.tight_layout()
        save_fig(fig, os.path.join(folder, 'timeseries_mem.png'))

    # 4. Peak utilization summary: peak, P95, median for CPU and IO at each SoI count
    if 'cpu_pct' in df.columns and 'io_util_pct' in df.columns:
        import numpy as np
        stats = []
        for s in soi_counts:
            sub = df[df['soi_workers'] == s]
            stats.append({
                'soi': s,
                'cpu_peak': sub['cpu_pct'].max(),
                'cpu_p95': sub['cpu_pct'].quantile(0.95),
                'cpu_median': sub['cpu_pct'].median(),
                'io_peak': sub['io_util_pct'].max(),
                'io_p95': sub['io_util_pct'].quantile(0.95),
                'io_median': sub['io_util_pct'].median(),
            })
        sdf = pd.DataFrame(stats)

        fig, (ax_cpu, ax_io) = plt.subplots(2, 1, figsize=(10, 7), sharex=True,
                                             layout='constrained')
        ax_cpu.fill_between(sdf['soi'], sdf['cpu_median'], sdf['cpu_peak'],
                            alpha=0.15, color=C_ORANGE)
        ax_cpu.fill_between(sdf['soi'], sdf['cpu_median'], sdf['cpu_p95'],
                            alpha=0.25, color=C_ORANGE)
        ax_cpu.plot(sdf['soi'], sdf['cpu_peak'], '-^', color=C_RED, linewidth=2,
                    markersize=6, label='Peak')
        ax_cpu.plot(sdf['soi'], sdf['cpu_p95'], '-s', color=C_ORANGE, linewidth=2,
                    markersize=5, label='P95')
        ax_cpu.plot(sdf['soi'], sdf['cpu_median'], '-o', color=C_BLUE, linewidth=2,
                    markersize=5, label='Median')
        ax_cpu.set_ylabel('CPU Utilization (%)')
        ax_cpu.set_ylim(0, 105)
        ax_cpu.legend(loc='best', fontsize=9)
        ax_cpu.grid(True, alpha=0.3)
        ax_cpu.set_title(f'SoI Sweep ({soi_type}) — Utilization Distribution (Peak / P95 / Median)')

        ax_io.fill_between(sdf['soi'], sdf['io_median'], sdf['io_peak'],
                           alpha=0.15, color=C_GREEN)
        ax_io.fill_between(sdf['soi'], sdf['io_median'], sdf['io_p95'],
                           alpha=0.25, color=C_GREEN)
        ax_io.plot(sdf['soi'], sdf['io_peak'], '-^', color=C_RED, linewidth=2,
                   markersize=6, label='Peak')
        ax_io.plot(sdf['soi'], sdf['io_p95'], '-s', color=C_GREEN, linewidth=2,
                   markersize=5, label='P95')
        ax_io.plot(sdf['soi'], sdf['io_median'], '-o', color=C_BLUE, linewidth=2,
                   markersize=5, label='Median')
        ax_io.set_xlabel('SoI Workers')
        ax_io.set_ylabel('IO BW Utilization (%)')
        ax_io.set_ylim(0, 105)
        ax_io.legend(loc='best', fontsize=9)
        ax_io.grid(True, alpha=0.3)
        ax_io.xaxis.set_major_locator(MaxNLocator(integer=True))
        save_fig(fig, os.path.join(folder, 'utilization_distribution.png'))

    # 5. Utilization CDF at baseline (0 SoI) — shows what fraction of time each resource
    #    spends at various utilization levels
    if 'cpu_pct' in df.columns and 'io_util_pct' in df.columns:
        baseline = df[df['soi_workers'] == soi_counts[0]]
        if len(baseline) > 5:
            fig, ax = plt.subplots(figsize=(8, 5))
            for col, color, label in [('cpu_pct', C_ORANGE, 'CPU'),
                                       ('io_util_pct', C_GREEN, 'IO BW')]:
                sorted_vals = np.sort(baseline[col].values)
                cdf = np.arange(1, len(sorted_vals) + 1) / len(sorted_vals)
                ax.plot(sorted_vals, cdf * 100, '-', color=color, linewidth=2, label=label)
            ax.set_xlabel('Utilization (%)')
            ax.set_ylabel('Percentile')
            ax.set_title(f'SoI Sweep ({soi_type}) — Baseline Utilization CDF (0 SoI workers)')
            ax.set_xlim(0, 105)
            ax.set_ylim(0, 100)
            ax.legend(loc='best', fontsize=9)
            ax.grid(True, alpha=0.3)
            save_fig(fig, os.path.join(folder, 'utilization_cdf_baseline.png'))


def plot_ext_saturation_timeseries(csv_path: str):
    """Generate time-series plots for external workload saturation experiments."""
    df = pd.read_csv(csv_path)
    folder = str(Path(csv_path).parent)

    concurrencies = sorted(df['concurrency'].unique())

    print(f"  -> {folder}/")

    # 1. External throughput over time, one line per concurrency level
    fig, ax = plt.subplots(figsize=(10, 6))
    cmap = plt.cm.viridis
    for i, n in enumerate(concurrencies):
        sub = df[df['concurrency'] == n]
        color = cmap(i / max(len(concurrencies) - 1, 1))
        ax.plot(sub['elapsed_ms'] / 1000, sub['ext_ops_sec'], '-', color=color,
                linewidth=1.2, alpha=0.8, label=f'N={int(n)}')
    ax.set_xlabel('Elapsed (seconds)')
    ax.set_ylabel('External Throughput (ops/sec)')
    ax.set_title('External Workload Saturation — Throughput Time Series')
    ax.set_ylim(bottom=0)
    ax.legend(loc='best', fontsize=8, ncol=2)
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'timeseries_ext_throughput.png'))

    # 2. Heatmap: external throughput over time × concurrency
    if len(concurrencies) > 1:
        pivot = df.pivot_table(index='concurrency', columns='elapsed_ms', values='ext_ops_sec')
        fig, ax = plt.subplots(figsize=(12, 5))
        im = ax.imshow(pivot.values, aspect='auto', cmap='RdYlGn',
                       extent=[pivot.columns.min() / 1000, pivot.columns.max() / 1000,
                               pivot.index.max() + 0.5, pivot.index.min() - 0.5])
        ax.set_xlabel('Elapsed (seconds)')
        ax.set_ylabel('Concurrency (N)')
        ax.set_title('External Workload Saturation — Throughput Heatmap')
        ax.set_yticks(concurrencies)
        cbar = fig.colorbar(im, ax=ax, label='External ops/sec')
        cbar.formatter = plt.FuncFormatter(lambda v, p: format_number(v))
        cbar.update_ticks()
        save_fig(fig, os.path.join(folder, 'timeseries_heatmap.png'))

    # 3. CPU utilization over time — small multiples
    if 'cpu_pct' in df.columns:
        n = len(concurrencies)
        fig, axes = plt.subplots(n, 1, figsize=(12, 1.8 * n), sharex=True, sharey=True)
        if n == 1:
            axes = [axes]
        for i, (ax, c) in enumerate(zip(axes, concurrencies)):
            sub = df[df['concurrency'] == c]
            color = cmap(i / max(n - 1, 1))
            ax.plot(sub['elapsed_ms'] / 1000, sub['cpu_pct'], '-', color=color,
                    linewidth=1.0, alpha=0.8)
            ax.set_ylabel(f'N={int(c)}', fontsize=9, rotation=0, labelpad=35, va='center')
            ax.set_ylim(0, 105)
            ax.grid(True, alpha=0.3)
            ax.tick_params(axis='y', labelsize=7)
        axes[-1].set_xlabel('Elapsed (seconds)')
        fig.suptitle('External Workload Saturation — CPU Utilization Time Series', fontsize=12)
        fig.tight_layout()
        save_fig(fig, os.path.join(folder, 'timeseries_cpu.png'))

    # 4. IO BW utilization over time — small multiples
    if 'io_util_pct' in df.columns:
        n = len(concurrencies)
        fig, axes = plt.subplots(n, 1, figsize=(12, 1.8 * n), sharex=True, sharey=True)
        if n == 1:
            axes = [axes]
        for i, (ax, c) in enumerate(zip(axes, concurrencies)):
            sub = df[df['concurrency'] == c]
            color = cmap(i / max(n - 1, 1))
            ax.plot(sub['elapsed_ms'] / 1000, sub['io_util_pct'], '-', color=color,
                    linewidth=1.0, alpha=0.8)
            ax.set_ylabel(f'N={int(c)}', fontsize=9, rotation=0, labelpad=35, va='center')
            ax.set_ylim(0, 105)
            ax.grid(True, alpha=0.3)
            ax.tick_params(axis='y', labelsize=7)
        axes[-1].set_xlabel('Elapsed (seconds)')
        fig.suptitle('External Workload Saturation — IO BW Utilization Time Series', fontsize=12)
        fig.tight_layout()
        save_fig(fig, os.path.join(folder, 'timeseries_io_bw.png'))

    # 5. Memory utilization over time — small multiples
    if 'mem_usage_pct' in df.columns:
        n = len(concurrencies)
        fig, axes = plt.subplots(n, 1, figsize=(12, 1.8 * n), sharex=True, sharey=True)
        if n == 1:
            axes = [axes]
        for i, (ax, c) in enumerate(zip(axes, concurrencies)):
            sub = df[df['concurrency'] == c]
            color = cmap(i / max(n - 1, 1))
            ax.plot(sub['elapsed_ms'] / 1000, sub['mem_usage_pct'], '-', color=color,
                    linewidth=1.0, alpha=0.8)
            ax.set_ylabel(f'N={int(c)}', fontsize=9, rotation=0, labelpad=35, va='center')
            ax.set_ylim(0, 105)
            ax.grid(True, alpha=0.3)
            ax.tick_params(axis='y', labelsize=7)
        axes[-1].set_xlabel('Elapsed (seconds)')
        fig.suptitle('External Workload Saturation — Memory Utilization Time Series', fontsize=12)
        fig.tight_layout()
        save_fig(fig, os.path.join(folder, 'timeseries_mem.png'))


def plot_ext_saturation(csv_path: str):
    """Generate graphs for external workload saturation CSV."""
    df = pd.read_csv(csv_path)
    folder = str(Path(csv_path).parent)
    x = df['concurrency']

    has_stddev = 'ext_ops_stddev' in df.columns

    print(f"  -> {folder}/")

    peak_idx = df['ext_ops_sec'].idxmax()
    peak_n = df.loc[peak_idx, 'concurrency']
    peak_ops = df.loc[peak_idx, 'ext_ops_sec']

    # 1. Throughput vs concurrency
    fig, ax = plt.subplots(figsize=(8, 5))
    ax.errorbar(x, df['ext_ops_sec'],
                yerr=df['ext_ops_stddev'] if has_stddev else None,
                fmt='-o', color=C_BLUE, linewidth=2, markersize=6,
                capsize=3, capthick=1, label='External ops/s')
    ax.axvline(x=peak_n, color=C_RED, linestyle='--', alpha=0.7,
               label=f'Peak: N={int(peak_n)}')
    ax.scatter([peak_n], [peak_ops], color=C_RED, s=100, zorder=5)
    ax.set_xlabel('Concurrency (N)')
    ax.set_ylabel('Throughput (ops/sec)')
    ax.set_title('External Workload Saturation')
    ax.set_ylim(bottom=0)
    ax.set_xlim(left=1)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.legend()
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'throughput_total.png'))

    # 2. Throughput per unit (efficiency)
    fig, ax = plt.subplots(figsize=(8, 5))
    ax.plot(x, df['throughput_per_unit'], '-s', color=C_BLUE, linewidth=2, markersize=6)
    ax.set_xlabel('Concurrency (N)')
    ax.set_ylabel('Throughput per Unit (ops/sec)')
    ax.set_title('External Workload — Per-Unit Efficiency')
    ax.set_ylim(bottom=0)
    ax.set_xlim(left=1)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'throughput_per_unit.png'))

    # 3. Utilization
    if 'cpu_pct' in df.columns:
        fig, ax = plt.subplots(figsize=(8, 5))
        ax.plot(x, df['cpu_pct'], '-o', color=C_ORANGE, linewidth=2, markersize=6, label='CPU %')
        if 'io_util_pct' in df.columns:
            ax.plot(x, df['io_util_pct'], '-s', color=C_GREEN, linewidth=2, markersize=6, label='IO BW %')
        if 'io_iops_util_pct' in df.columns:
            ax.plot(x, df['io_iops_util_pct'], '-^', color=C_CYAN, linewidth=2, markersize=6, label='IO IOPS %')
        if 'mem_usage_pct' in df.columns:
            ax.plot(x, df['mem_usage_pct'], '-D', color=C_PURPLE, linewidth=2, markersize=6, label='Memory %')
        ax.set_xlabel('Concurrency (N)')
        ax.set_ylabel('Utilization (%)')
        ax.set_title('External Workload Saturation — Resource Utilization')
        ax.set_ylim(0, 105)
        ax.set_xlim(left=1)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend()
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'utilization.png'))


def plot_ext_soi(csv_path: str):
    """Generate graphs for external workload SoI sweep CSV."""
    df = pd.read_csv(csv_path)
    name = Path(csv_path).stem
    folder = str(Path(csv_path).parent)
    x = df['soi_workers']

    # Extract SoI type from filename: ext_soi_<type>_throughput
    soi_type = name.replace('ext_soi_', '').replace('_throughput', '')

    has_stddev = 'ext_ops_stddev' in df.columns

    print(f"  -> {folder}/")

    # 1. External workload throughput vs SoI workers
    fig, ax = plt.subplots(figsize=(8, 5))
    ax.errorbar(x, df['ext_ops_sec'],
                yerr=df['ext_ops_stddev'] if has_stddev else None,
                fmt='-o', color=C_BLUE, linewidth=2, markersize=6,
                capsize=3, capthick=1, label='External ops/s')
    ax.set_xlabel('SoI Workers')
    ax.set_ylabel('External Throughput (ops/sec)')
    ax.set_title(f'SoI Sweep ({soi_type}) — External Workload Throughput')
    ax.set_ylim(bottom=0)
    ax.set_xlim(left=0)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.legend(loc='best', fontsize=9)
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'victim_throughput.png'))

    # 2. Degradation percentage
    fig, ax = plt.subplots(figsize=(8, 5))
    ax.plot(x, df['ext_change_pct'], '-o', color=C_RED, linewidth=2, markersize=5)
    ax.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
    ax.fill_between(x, df['ext_change_pct'], 0,
                    where=(df['ext_change_pct'] < 0), interpolate=True, alpha=0.2, color=C_RED)
    ax.set_xlabel('SoI Workers')
    ax.set_ylabel('Throughput Change (%)')
    ax.set_title(f'SoI Sweep ({soi_type}) — External Workload Degradation')
    ax.set_xlim(left=0)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'victim_degradation.png'))

    # 3. SoI throughput
    if df['soi_ops'].max() > 0:
        fig, ax = plt.subplots(figsize=(8, 5))
        ax.plot(x[x > 0], df.loc[x > 0, 'soi_ops'], '-^', color=C_ORANGE,
                linewidth=2, markersize=6, label='SoI ops/s')
        ax.set_xlabel('SoI Workers')
        ax.set_ylabel('SoI Throughput (ops/sec)')
        ax.set_title(f'SoI Sweep ({soi_type}) — SoI Throughput')
        ax.set_ylim(bottom=0)
        ax.set_xlim(left=0)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend(loc='best', fontsize=9)
        ax.grid(True, alpha=0.3)
        ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
        save_fig(fig, os.path.join(folder, 'soi_throughput.png'))

    # 4. Combined: throughput + degradation on dual axes
    fig, ax1 = plt.subplots(figsize=(8, 5))
    line_tp = ax1.plot(x, df['ext_ops_sec'], '-o', color=C_BLUE, linewidth=2,
                       markersize=6, label='External ops/s')
    ax1.set_xlabel('SoI Workers')
    ax1.set_ylabel('External Throughput (ops/sec)', color=C_BLUE)
    ax1.tick_params(axis='y', labelcolor=C_BLUE)
    ax1.set_ylim(bottom=0)
    ax1.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))

    ax2 = ax1.twinx()
    line_deg = ax2.plot(x, df['ext_change_pct'], '-s', color=C_RED, linewidth=2,
                        markersize=5, label='Change %')
    ax2.set_ylabel('Throughput Change (%)', color=C_RED)
    ax2.tick_params(axis='y', labelcolor=C_RED)

    lines = line_tp + line_deg
    ax1.legend(lines, [l.get_label() for l in lines], loc='best')
    ax1.set_title(f'SoI Sweep ({soi_type}) — Throughput vs Degradation')
    ax1.set_xlim(left=0)
    ax1.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax1.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(folder, 'throughput_vs_degradation.png'))

    # 5. Resource utilization
    if 'cpu_pct' in df.columns:
        fig, ax = plt.subplots(figsize=(8, 5))
        ax.plot(x, df['cpu_pct'], '-o', color=C_ORANGE, linewidth=2, markersize=6, label='CPU %')
        if 'io_util_pct' in df.columns:
            ax.plot(x, df['io_util_pct'], '-s', color=C_GREEN, linewidth=2, markersize=6, label='IO BW %')
        if 'io_iops_util_pct' in df.columns:
            ax.plot(x, df['io_iops_util_pct'], '-^', color=C_CYAN, linewidth=2, markersize=6, label='IO IOPS %')
        if 'mem_usage_pct' in df.columns:
            ax.plot(x, df['mem_usage_pct'], '-D', color=C_PURPLE, linewidth=2, markersize=6, label='Memory %')
        ax.set_xlabel('SoI Workers')
        ax.set_ylabel('Utilization (%)')
        ax.set_title(f'SoI Sweep ({soi_type}) — Resource Utilization')
        ax.set_ylim(0, 105)
        ax.set_xlim(left=0)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend()
        ax.grid(True, alpha=0.3)
        save_fig(fig, os.path.join(folder, 'utilization.png'))


def plot_per_worker_ext_soi(csv_path: str):
    """Box plot of SoI worker throughput distribution for external workload experiments."""
    df = pd.read_csv(csv_path)
    folder = str(Path(csv_path).parent)

    soi_counts = sorted(df['soi_workers'].unique())

    print(f"  -> {folder}/")

    box_width = (soi_counts[1] - soi_counts[0]) * 0.6 if len(soi_counts) > 1 else 0.6

    # Box plot of SoI worker throughput distribution
    fig, ax = plt.subplots(figsize=(max(8, len(soi_counts) * 0.5), 5))
    data = [df[df['soi_workers'] == s]['total_ops_sec'].values for s in soi_counts]
    ax.boxplot(data, positions=soi_counts, widths=box_width * 0.7, patch_artist=True,
               boxprops=dict(facecolor=C_CYAN, alpha=0.7),
               medianprops=dict(color=C_BLUE, linewidth=2), manage_ticks=False)
    ax.set_xticks(soi_counts)
    ax.set_xticklabels([int(s) for s in soi_counts])
    ax.set_xlabel('SoI Workers')
    ax.set_ylabel('Per-Worker SoI Throughput (ops/sec)')
    ax.set_title('External SoI Sweep — SoI Worker Distribution')
    ax.set_ylim(bottom=0)
    ax.grid(True, alpha=0.3)
    ax.yaxis.set_major_formatter(plt.FuncFormatter(lambda v, p: format_number(v)))
    save_fig(fig, os.path.join(folder, 'per_worker_distribution.png'))


def plot_soi_comparison(soi_csvs: list):
    """Generate comparison plots across multiple SoI types."""
    if len(soi_csvs) < 2:
        return

    # Sort by SoI type name for consistent ordering
    soi_csvs = sorted(soi_csvs, key=lambda p: pd.read_csv(p, nrows=1)['soi_type'].iloc[0])

    # Severity ordering for legend (most impactful last so it's visually on top)
    SOI_COLORS = {
        'cpu': C_BLUE, 'l1d': C_CYAN, 'l2': C_PURPLE, 'l3': C_GREEN,
        'membw': C_ORANGE, 'memcap': '#999999', 'iobw': C_RED, 'iops': '#8B4513',
    }
    SOI_MARKERS = {
        'cpu': 'o', 'l1d': 's', 'l2': '^', 'l3': 'D',
        'membw': 'v', 'memcap': 'x', 'iobw': 'P', 'iops': '*',
    }

    # Load all data
    entries = []
    for csv_path in soi_csvs:
        df = pd.read_csv(csv_path)
        soi_type = df['soi_type'].iloc[0]
        if soi_type == 'ioops':
            soi_type = 'iops'
        entries.append((soi_type, df, csv_path))

    # Use the parent of the first CSV's parent as output dir
    out_dir = str(Path(soi_csvs[0]).parent.parent)
    victim_workers = int(entries[0][1]['victim_workers'].iloc[0])

    print(f"  SoI comparison -> {out_dir}/")

    # Derive split change pcts from raw ops if not in CSV
    for i, (soi_type, df, csv_path) in enumerate(entries):
        if 'victim_cpu_change_pct' not in df.columns and 'victim_cpu_ops' in df.columns:
            base_cpu = df['victim_cpu_ops'].iloc[0]
            base_io = df['victim_io_ops'].iloc[0]
            df['victim_cpu_change_pct'] = ((df['victim_cpu_ops'] - base_cpu) / base_cpu * 100.0) if base_cpu > 0 else 0.0
            df['victim_io_change_pct'] = ((df['victim_io_ops'] - base_io) / base_io * 100.0) if base_io > 0 else 0.0
            entries[i] = (soi_type, df, csv_path)

    # Check if split degradation columns are available
    has_split_change = all('victim_cpu_change_pct' in df.columns and 'victim_io_change_pct' in df.columns
                           for _, df, _ in entries)
    # Detect if this is a mixed workload (both CPU and IO ops present)
    is_mixed = has_split_change and all(
        df['victim_cpu_ops'].max() > 0 and df['victim_io_ops'].max() > 0
        for _, df, _ in entries
    )

    # 1. Victim degradation comparison (all types on one plot)
    if is_mixed:
        fig, (ax_cpu, ax_io) = plt.subplots(2, 1, figsize=(10, 8), sharex=True, layout='constrained')
        for soi_type, df, _ in entries:
            color = SOI_COLORS.get(soi_type, 'gray')
            marker = SOI_MARKERS.get(soi_type, 'o')
            ax_cpu.plot(df['soi_workers'], df['victim_cpu_change_pct'], f'-{marker}',
                        color=color, linewidth=2, markersize=6, label=soi_type)
            ax_io.plot(df['soi_workers'], df['victim_io_change_pct'], f'-{marker}',
                       color=color, linewidth=2, markersize=6, label=soi_type)
        ax_cpu.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
        ax_cpu.set_ylabel('CPU Throughput Change (%)')
        ax_cpu.legend(loc='best', ncol=2, fontsize=9)
        ax_cpu.grid(True, alpha=0.3)
        ax_io.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
        ax_io.set_xlabel('SoI Workers')
        ax_io.set_ylabel('IO Throughput Change (%)')
        ax_io.legend(loc='best', ncol=2, fontsize=9)
        ax_io.grid(True, alpha=0.3)
        fig.suptitle(f'SoI Comparison — Victim Degradation ({victim_workers} victims)')
    else:
        fig, ax = plt.subplots(figsize=(10, 6))
        for soi_type, df, _ in entries:
            color = SOI_COLORS.get(soi_type, 'gray')
            marker = SOI_MARKERS.get(soi_type, 'o')
            ax.plot(df['soi_workers'], df['victim_change_pct'], f'-{marker}',
                    color=color, linewidth=2, markersize=6, label=soi_type)
        ax.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
        ax.set_xlabel('SoI Workers')
        ax.set_ylabel('Victim Throughput Change (%)')
        ax.set_title(f'SoI Comparison — Victim Degradation ({victim_workers} victims)')
        ax.set_xlim(left=0)
        ax.xaxis.set_major_locator(MaxNLocator(integer=True))
        ax.legend(loc='best', ncol=2, fontsize=9)
        ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(out_dir, 'soi_comparison_degradation.png'))

    # 2. Bar chart of final degradation at max SoI workers
    if is_mixed:
        fig, (ax_cpu, ax_io) = plt.subplots(1, 2, figsize=(14, 5), layout='constrained')
        for ax_bar, change_col, title_label in [
            (ax_cpu, 'victim_cpu_change_pct', 'CPU'),
            (ax_io, 'victim_io_change_pct', 'IO'),
        ]:
            types = []
            final_degs = []
            colors = []
            for soi_type, df, _ in entries:
                types.append(soi_type)
                final_degs.append(df[change_col].iloc[-1])
                colors.append(SOI_COLORS.get(soi_type, 'gray'))
            order = sorted(range(len(final_degs)), key=lambda i: final_degs[i])
            types = [types[i] for i in order]
            final_degs = [final_degs[i] for i in order]
            colors = [colors[i] for i in order]
            bars = ax_bar.bar(types, final_degs, color=colors, alpha=0.8, edgecolor='black', linewidth=0.5)
            ax_bar.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
            for bar, val in zip(bars, final_degs):
                ax_bar.text(bar.get_x() + bar.get_width() / 2, val - 1.5,
                            f'{val:.1f}%', ha='center', va='top', fontsize=9, fontweight='bold')
            ax_bar.set_xlabel('SoI Type')
            ax_bar.set_ylabel(f'{title_label} Throughput Change (%)')
            ax_bar.set_title(f'{title_label} Impact')
            ax_bar.grid(True, alpha=0.3, axis='y')
        max_soi = int(entries[0][1]['soi_workers'].max())
        fig.suptitle(f'SoI Impact — {max_soi} SoI Workers ({victim_workers} victims)')
    else:
        fig, ax = plt.subplots(figsize=(10, 5))
        types = []
        final_degs = []
        colors = []
        for soi_type, df, _ in entries:
            types.append(soi_type)
            final_degs.append(df['victim_change_pct'].iloc[-1])
            colors.append(SOI_COLORS.get(soi_type, 'gray'))
        # Sort by degradation severity
        order = sorted(range(len(final_degs)), key=lambda i: final_degs[i])
        types = [types[i] for i in order]
        final_degs = [final_degs[i] for i in order]
        colors = [colors[i] for i in order]

        bars = ax.bar(types, final_degs, color=colors, alpha=0.8, edgecolor='black', linewidth=0.5)
        ax.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
        for bar, val in zip(bars, final_degs):
            ax.text(bar.get_x() + bar.get_width() / 2, val - 1.5,
                    f'{val:.1f}%', ha='center', va='top', fontsize=9, fontweight='bold')
        ax.set_xlabel('SoI Type')
        ax.set_ylabel('Victim Throughput Change (%)')
        max_soi = int(entries[0][1]['soi_workers'].max())
        ax.set_title(f'SoI Impact — {max_soi} SoI Workers ({victim_workers} victims)')
        ax.grid(True, alpha=0.3, axis='y')
    save_fig(fig, os.path.join(out_dir, 'soi_comparison_bar.png'))

    # 3. Victim throughput (normalized % of baseline) comparison
    fig, ax = plt.subplots(figsize=(10, 6))
    for soi_type, df, _ in entries:
        color = SOI_COLORS.get(soi_type, 'gray')
        marker = SOI_MARKERS.get(soi_type, 'o')
        baseline = df['victim_total_ops'].iloc[0]
        if baseline > 0:
            normalized = df['victim_total_ops'] / baseline * 100
            ax.plot(df['soi_workers'], normalized, f'-{marker}',
                    color=color, linewidth=2, markersize=6, label=soi_type)
    ax.axhline(y=100, color='gray', linestyle='--', alpha=0.5, label='Baseline')
    ax.set_xlabel('SoI Workers')
    ax.set_ylabel('Victim Throughput (% of baseline)')
    ax.set_title(f'SoI Comparison — Normalized Victim Throughput ({victim_workers} victims)')
    ax.set_xlim(left=0)
    ax.set_ylim(bottom=0)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.legend(loc='best', ncol=2, fontsize=9)
    ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(out_dir, 'soi_comparison_normalized.png'))


def plot_ext_soi_comparison(ext_soi_csvs: list):
    """Generate comparison plots across multiple external SoI types."""
    if len(ext_soi_csvs) < 2:
        return

    SOI_COLORS = {
        'cpu': C_BLUE, 'l1d': C_CYAN, 'l2': C_PURPLE, 'l3': C_GREEN,
        'membw': C_ORANGE, 'memcap': '#999999', 'iobw': C_RED, 'iops': '#8B4513',
    }
    SOI_MARKERS = {
        'cpu': 'o', 'l1d': 's', 'l2': '^', 'l3': 'D',
        'membw': 'v', 'memcap': 'x', 'iobw': 'P', 'iops': '*',
    }

    entries = []
    for csv_path in ext_soi_csvs:
        df = pd.read_csv(csv_path)
        name = Path(csv_path).stem
        soi_type = name.replace('ext_soi_', '').replace('_throughput', '')
        entries.append((soi_type, df, csv_path))
    entries.sort(key=lambda e: e[0])

    out_dir = str(Path(ext_soi_csvs[0]).parent.parent)
    print(f"  Ext SoI comparison -> {out_dir}/")

    # 1. Degradation comparison
    fig, ax = plt.subplots(figsize=(10, 6))
    for soi_type, df, _ in entries:
        color = SOI_COLORS.get(soi_type, 'gray')
        marker = SOI_MARKERS.get(soi_type, 'o')
        ax.plot(df['soi_workers'], df['ext_change_pct'], f'-{marker}',
                color=color, linewidth=2, markersize=6, label=soi_type)
    ax.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
    ax.set_xlabel('SoI Workers')
    ax.set_ylabel('External Throughput Change (%)')
    ax.set_title('SoI Comparison — External Workload Degradation')
    ax.set_xlim(left=0)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.legend(loc='best', ncol=2, fontsize=9)
    ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(out_dir, 'soi_comparison_degradation.png'))

    # 2. Bar chart of final degradation at max SoI workers
    fig, ax = plt.subplots(figsize=(10, 5))
    types = []
    final_degs = []
    colors = []
    for soi_type, df, _ in entries:
        types.append(soi_type)
        final_degs.append(df['ext_change_pct'].iloc[-1])
        colors.append(SOI_COLORS.get(soi_type, 'gray'))
    order = sorted(range(len(final_degs)), key=lambda i: final_degs[i])
    types = [types[i] for i in order]
    final_degs = [final_degs[i] for i in order]
    colors = [colors[i] for i in order]
    bars = ax.bar(types, final_degs, color=colors, alpha=0.8, edgecolor='black', linewidth=0.5)
    ax.axhline(y=0, color='gray', linestyle='-', alpha=0.5)
    for bar, val in zip(bars, final_degs):
        y_pos = val - 1.5 if val < 0 else val + 0.5
        va = 'top' if val < 0 else 'bottom'
        ax.text(bar.get_x() + bar.get_width() / 2, y_pos,
                f'{val:.1f}%', ha='center', va=va, fontsize=9, fontweight='bold')
    ax.set_xlabel('SoI Type')
    ax.set_ylabel('External Throughput Change (%)')
    max_soi = int(entries[0][1]['soi_workers'].max())
    ax.set_title(f'SoI Impact at {max_soi} Workers — External Workload')
    ax.grid(True, alpha=0.3, axis='y')
    save_fig(fig, os.path.join(out_dir, 'soi_comparison_bar.png'))

    # 3. Normalized throughput comparison
    fig, ax = plt.subplots(figsize=(10, 6))
    for soi_type, df, _ in entries:
        color = SOI_COLORS.get(soi_type, 'gray')
        marker = SOI_MARKERS.get(soi_type, 'o')
        baseline = df['ext_ops_sec'].iloc[0]
        if baseline > 0:
            normalized = df['ext_ops_sec'] / baseline * 100
            ax.plot(df['soi_workers'], normalized, f'-{marker}',
                    color=color, linewidth=2, markersize=6, label=soi_type)
    ax.axhline(y=100, color='gray', linestyle='--', alpha=0.5, label='Baseline')
    ax.set_xlabel('SoI Workers')
    ax.set_ylabel('External Throughput (% of baseline)')
    ax.set_title('SoI Comparison — Normalized External Throughput')
    ax.set_xlim(left=0)
    ax.set_ylim(bottom=0)
    ax.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax.legend(loc='best', ncol=2, fontsize=9)
    ax.grid(True, alpha=0.3)
    save_fig(fig, os.path.join(out_dir, 'soi_comparison_normalized.png'))


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
                elif name.startswith('per_worker_ext_soi_'):
                    plot_per_worker_ext_soi(csv_path)
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

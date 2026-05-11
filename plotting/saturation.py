"""Plots for thread/process saturation experiments."""

import os
from pathlib import Path

import matplotlib.pyplot as plt
import pandas as pd
from matplotlib.ticker import MaxNLocator

from .common import (
    C_BLUE, C_CYAN, C_GREEN, C_ORANGE, C_PURPLE, C_RED,
    detect_experiment, format_number, save_fig,
)


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

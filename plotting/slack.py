"""Plots for slack experiments (thread + process modes)."""

import os
from pathlib import Path

import matplotlib.pyplot as plt
import pandas as pd
from matplotlib.ticker import MaxNLocator

from .common import (
    C_BLUE, C_CYAN, C_GREEN, C_ORANGE, C_PURPLE, C_RED,
    format_number, save_fig,
)


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
